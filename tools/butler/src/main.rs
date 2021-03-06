//! `butler` is Pebble's high-level build tool, coordinating the building of many different components which
//! eventually fit together to form an operating system. It can be used to build and test a Pebble distribution,
//! emulate it on the host, and pave it onto a real device (WIP).
//!
//! At the core of `butler` is the idea of Projects. Each project can build and assemble a number of components
//! (using the `build` subcommand) and can be run in some way (using the `run` subcommand).

/*
 * TODO:
 *    - `update_submodules` subcommand that goes through each submodule, looks at git status, pulls it if clean,
 *    presents list at end (color coded!!!) for status of each one - ([DIRTY], [UP TO DATE], [UPDATED], [REMOTE
 *    MISSING!!])
 *    - `rust` subcommand that can build a custom Rust toolchain that includes all the correct Pebble stuff
 *    - A central list of crates for when things should be done to every crate in the tree
 *    - `test` subcommand to run tests on all crates that can be tested on the host
 *    - `clean` subcommand to clean up everything left behind by the build process
 */

#![feature(bool_to_option, type_ascription, unsized_fn_params)]

mod build;
mod qemu;

use build::{
    cargo::{RunCargo, Target},
    image::MakeGptImage,
    BuildStep,
    MakeDirectories,
};
use clap::{App, Arg};
use eyre::Result;
use qemu::{QemuOptions, RunQemuX64};
use std::{path::PathBuf, string::ToString};

/// A Project is something that you can instruct Butler to build or run. This might be a Pebble distribution, or
/// something else (e.g. a target-side test that doesn't use the Pebble kernel).
pub struct Project {
    name: String,
    build_steps: Vec<Box<dyn BuildStep>>,
    // TODO: abstract
    pub qemu: Option<RunQemuX64>,
}

impl Project {
    pub fn new(name: String) -> Project {
        Project { name, build_steps: Vec::new(), qemu: None }
    }

    pub fn add_build_step<T>(&mut self, step: T)
    where
        T: BuildStep + 'static,
    {
        self.build_steps.push(Box::new(step));
    }

    pub fn build(&mut self) {
        for step in self.build_steps.drain(..) {
            // TODO: print a nice colored heading for each step, making it easy to see what's going on
            // e.g. '[1/34] Building Cargo project at kernel/efiloader
            match step.build() {
                Ok(_) => (),
                Err(err) => panic!("Build of project {} failed: {:?}", self.name, err),
            }
        }
    }

    pub fn run(self) {
        self.qemu.unwrap().run().unwrap()
    }
}

pub fn main() -> Result<()> {
    color_eyre::install()?;

    let matches = App::new("Butler")
        .version("0.1.0")
        .about("Host-side program for managing Pebble builds")
        .after_help(EXTRA_HELP)
        .subcommand(App::new("build").about("Build a project").arg(Arg::from_usage("[project]")))
        .subcommand(App::new("run").about("Build and run a project").arg(Arg::from_usage("[project]")))
        .get_matches();

    if let Some(sub_matches) = matches.subcommand_matches("build") {
        project_from_name(sub_matches.value_of("project")).build();
    } else if let Some(sub_matches) = matches.subcommand_matches("run") {
        let mut project = project_from_name(sub_matches.value_of("project"));
        project.build();
        project.run();
    } else {
        /*
         * If no subcommand is supplied, just build and run a normal Pebble distribution.
         */
        let mut pebble = pebble();
        pebble.build();
        pebble.run();
    }

    Ok(())
}

fn project_from_name(name: Option<&str>) -> Project {
    match name {
        Some("pebble") | None => pebble(),
        Some(other) => panic!("Unknown project name: {}", other),
    }
}

// TODO: Abstract this out into a "Pebble build"-builder sort of thing
// TODO: to build a full-std user task, use `-Zbuild-std=core,alloc,panic_abort,std -Zbuild-std-features=compiler-builtins-mem`
fn pebble() -> Project {
    let build_dir = PathBuf::from("build/Pebble");
    let release = false;

    let mut pebble = Project::new("Pebble".to_string());
    // TODO: it would be nice to not need to copy each artifact out of its target folder, and instead just know the
    // correct paths to put into the GPT step
    pebble.add_build_step(MakeDirectories(build_dir.join("fat/efi/boot/")));
    pebble.add_build_step(RunCargo {
        toolchain: None,
        manifest_path: PathBuf::from("kernel/efiloader/Cargo.toml"),
        target: Target::Triple("x86_64-unknown-uefi".to_string()),
        workspace: PathBuf::from("kernel"),
        release,
        std_components: vec!["core".to_string()],
        std_features: vec!["compiler-builtins-mem".to_string()],
        artifact_name: "efiloader.efi".to_string(),
        artifact_path: Some(build_dir.join("fat/efi/boot/bootx64.efi")),
    });
    pebble.add_build_step(RunCargo {
        toolchain: None,
        manifest_path: PathBuf::from("kernel/kernel_x86_64/Cargo.toml"),
        target: Target::Custom {
            triple: "x86_64-kernel".to_string(),
            spec: PathBuf::from("kernel/kernel_x86_64/x86_64-kernel.json"),
        },
        workspace: PathBuf::from("kernel"),
        release,
        std_components: vec!["core".to_string(), "alloc".to_string()],
        std_features: vec![],
        artifact_name: "kernel_x86_64".to_string(),
        artifact_path: Some(build_dir.join("fat/kernel.elf")),
    });
    pebble.add_build_step(RunCargo {
        toolchain: Some("pebble".to_string()),
        manifest_path: PathBuf::from("user/test_tls/Cargo.toml"),
        target: Target::Triple("x86_64-pebble".to_string()),
        workspace: PathBuf::from("user"),
        release,
        std_components: vec!["core".to_string(), "alloc".to_string()],
        std_features: vec!["compiler-builtins-mem".to_string()],
        artifact_name: "test_tls".to_string(),
        artifact_path: Some(build_dir.join("fat/test_tls.elf")),
    });
    pebble.add_build_step(MakeGptImage {
        image_path: build_dir.join("pebble.img"),
        image_size: 30 * 1024 * 1024,
        efi_partition_size: 20 * 1024 * 1024,
        efi_part_files: vec![
            (String::from("efi/boot/bootx64.efi"), build_dir.join("fat/efi/boot/bootx64.efi")),
            (String::from("kernel.elf"), build_dir.join("fat/kernel.elf")),
            (String::from("test_tls.elf"), build_dir.join("fat/test_tls.elf")),
        ],
    });

    pebble.qemu = Some(RunQemuX64 {
        options: QemuOptions { ovmf_debugcon_to_file: true, ..Default::default() },
        image: build_dir.join("pebble.img"),
    });

    pebble
}

const EXTRA_HELP: &str = "Butler can build and run various projects.

Project list:
    - pebble                        This is the main Pebble distribution, and probably what you want.
";
