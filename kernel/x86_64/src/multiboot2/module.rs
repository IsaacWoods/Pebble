/*
 * Copyright (C) 2016, Philipp Oppermann.
 * Copyright (C) 2017, Pebble Developers.
 * See LICENCE.md
 */

use super::header::{Tag, TagIter};
use core::{mem, slice, str};
use memory::paging::PhysicalAddress;

#[repr(packed)]
#[derive(Clone, Copy, Debug)]
pub struct ModuleTag {
    typ: u32,
    size: u32,
    mod_start: u32,
    mod_end: u32,
    name_byte: u8,
}

impl ModuleTag {
    /*
     * The multiboot specification defines the module str
     * as valid utf-8, therefore this function produces
     * defined behavior
     */
    pub fn name(&self) -> &str {
        let length = self.size as usize - mem::size_of::<ModuleTag>();

        unsafe {
            str::from_utf8_unchecked(slice::from_raw_parts(&self.name_byte as *const u8, length))
        }
    }

    pub fn start_address(&self) -> PhysicalAddress {
        PhysicalAddress::new(self.mod_start as usize)
    }

    pub fn end_address(&self) -> PhysicalAddress {
        PhysicalAddress::new(self.mod_end as usize)
    }
}

pub fn module_iter(iter: TagIter) -> ModuleIter {
    ModuleIter { iter }
}

pub struct ModuleIter {
    iter: TagIter,
}

impl Iterator for ModuleIter {
    type Item = &'static ModuleTag;

    fn next(&mut self) -> Option<&'static ModuleTag> {
        self.iter
            .find(|x| x.typ == 3)
            .map(|tag| unsafe { &*(tag as *const Tag as *const ModuleTag) })
    }
}
