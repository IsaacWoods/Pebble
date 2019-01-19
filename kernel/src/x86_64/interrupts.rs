use super::Arch;
use crate::util::binary_pretty_print::BinaryPrettyPrint;
use acpi::interrupt::InterruptModel;
use log::{error, info};
use x86_64::hw::gdt::KERNEL_CODE_SELECTOR;
use x86_64::hw::i8259_pic::Pic;
use x86_64::hw::idt::{Idt, InterruptStackFrame};
use x86_64::hw::local_apic::LocalApic;
use x86_64::hw::registers::read_control_reg;
use x86_64::memory::paging::{entry::EntryFlags, Frame, Page};
use x86_64::memory::{kernel_map, PhysicalAddress};
/// This should only be accessed directly by the bootstrap processor.
///
/// The IDT is laid out like so:
/// |------------------|-----------------------------|
/// | Interrupt Vector |            Usage            |
/// |------------------|-----------------------------|
/// |       00-1F      | Intel Reserved (Exceptions) |
/// |       20-2F      | i8259 PIC Interrupts        |
/// |        30        | Local APIC timer            |
/// |        ..        |                             |
/// |       40-7f      | IOAPIC Interrupts           |
/// |        80        | Yield from usermode         |
/// |        ..        |                             |
/// |        FF        | APIC spurious interrupt     |
/// |------------------|-----------------------------|
static mut IDT: Idt = Idt::empty();

/*
 * These constants define the IDT's layout. Refer to the documentation of the `IDT` static for the
 * full layout.
 */
const LEGACY_PIC_VECTOR: u8 = 0x20;
const APIC_SPURIOUS_VECTOR: u8 = 0xff;
pub struct InterruptController {
}

impl InterruptController {
    pub fn init(arch: &Arch, interrupt_model: &InterruptModel) -> InterruptController {
        Self::install_exception_handlers();
        unsafe {
            IDT.load();
        }

        match interrupt_model {
            InterruptModel::Apic {
                local_apic_address,
                io_apics: acpi_io_apics,
                ref local_apic_nmi_line,
                ref interrupt_source_overrides,
                ref nmi_sources,
                also_has_legacy_pics,
            } => {
                if *also_has_legacy_pics {
                    let mut pic = unsafe { Pic::new() };
                    pic.remap_and_disable(LEGACY_PIC_VECTOR, LEGACY_PIC_VECTOR + 8);
                }

                /*
                 * Map the local APIC's configuration space into the kernel address space.
                 */
                arch.kernel_page_table.lock().map_to(
                    Page::contains(kernel_map::LOCAL_APIC_CONFIG),
                    Frame::contains(PhysicalAddress::new(*local_apic_address as usize).unwrap()),
                    EntryFlags::PRESENT
                        | EntryFlags::WRITABLE
                        | EntryFlags::NO_EXECUTE
                        | EntryFlags::NO_CACHE,
                    &arch.physical_memory_manager,
                );

                /*
                 * Install a spurious interrupt handler and enable the local APIC.
                 */
                unsafe {
                    IDT[APIC_SPURIOUS_VECTOR]
                        .set_handler(wrap_handler!(spurious_handler), KERNEL_CODE_SELECTOR);
                    LocalApic::enable(APIC_SPURIOUS_VECTOR);
                }

                // TODO: configure the local APIC timer
                InterruptController {}
            }

            _ => panic!("Unsupported interrupt model!"),
        }
    }

    fn install_exception_handlers() {
        macro set_handler($name: ident, $handler: ident) {
            unsafe {
                IDT.$name()
                    .set_handler(wrap_handler!($handler), KERNEL_CODE_SELECTOR);
            }
        }

        macro set_handler_with_error_code($name: ident, $handler: ident) {
            unsafe {
                IDT.$name().set_handler(
                    wrap_handler_with_error_code!($handler),
                    KERNEL_CODE_SELECTOR,
                );
            }
        }

        set_handler!(nmi, nmi_handler);
        set_handler!(breakpoint, breakpoint_handler);
        set_handler!(invalid_opcode, invalid_opcode_handler);
        set_handler_with_error_code!(general_protection_fault, general_protection_fault_handler);
        set_handler_with_error_code!(page_fault, page_fault_handler);
        set_handler_with_error_code!(double_fault, double_fault_handler);
    }
}

extern "C" fn nmi_handler(_: &InterruptStackFrame) {
    info!("NMI occured!");
}

extern "C" fn breakpoint_handler(stack_frame: &InterruptStackFrame) {
    info!("BREAKPOINT: {:#?}", stack_frame);
}

extern "C" fn invalid_opcode_handler(stack_frame: &InterruptStackFrame) {
    error!("INVALID OPCODE AT: {:#x}", stack_frame.instruction_pointer);

    loop {}
}

extern "C" fn general_protection_fault_handler(stack_frame: &InterruptStackFrame, error_code: u64) {
    error!(
        "General protection fault (error code = {:#x}). Interrupt stack frame: ",
        error_code
    );
    error!("{:#?}", stack_frame);

    loop {}
}

extern "C" fn page_fault_handler(stack_frame: &InterruptStackFrame, error_code: u64) {
    error!(
        "PAGE_FAULT: {} ({:#x})",
        match (
            /* U/S (User/Supervisor )*/ (error_code >> 2) & 0b1,
            /* I/D (Instruction/Data)*/ (error_code >> 4) & 0b1,
            /* R/W (Read/Write      )*/ (error_code >> 1) & 0b1,
            /*  P  (Present         )*/ (error_code >> 0) & 0b1
        ) {
            // Page faults caused by the kernel
            (0, 0, 0, 0) => "Kernel read non-present page",
            (0, 0, 0, 1) => "Kernel read present page",
            (0, 0, 1, 0) => "Kernel wrote to non-present page",
            (0, 0, 1, 1) => "Kernel wrote to present page",
            (0, 1, _, 0) => "Kernel fetched instruction from non-present page",
            (0, 1, _, 1) => "Kernel fetched instruction from present page",

            // Page faults caused by user processes
            (1, 0, 0, 0) => "User process read non-present page",
            (1, 0, 0, 1) => "User process read present page (probable access violation)",
            (1, 0, 1, 0) => "User process wrote to non-present page",
            (1, 0, 1, 1) => "User process wrote to present page (probable access violation)",
            (1, 1, _, 0) => "User process fetched instruction from non-present page",
            (1, 1, _, 1) => {
                "User process fetched instruction from present page (probable access violation)"
            }

            (_, _, _, _) => {
                panic!("INVALID PAGE-FAULT ERROR CODE");
            }
        },
        read_control_reg!(cr2) // CR2 holds the address of the page that caused the #PF
    );

    error!("Error code: {:?}", BinaryPrettyPrint(error_code));
    error!("{:#?}", stack_frame);

    /*
     * Page-faults can be recovered from and so are faults, but we never will so just give up.
     */
    loop {}
}

extern "C" fn double_fault_handler(stack_frame: &InterruptStackFrame, error_code: u64) {
    error!(
        "EXCEPTION: DOUBLE FAULT   (Error code: {})\n{:#?}",
        error_code, stack_frame
    );

    loop {}
}

extern "C" fn spurious_handler(_: &InterruptStackFrame) {}
macro save_regs() {
    asm!("push rax
          push rcx
          push rdx
          push rsi
          push rdi
          push r8
          push r9
          push r10
          push r11"
        :
        :
        :
        : "intel"
        );
}

macro restore_regs() {
    asm!("pop r11
          pop r10
          pop r9
          pop r8
          pop rdi
          pop rsi
          pop rdx
          pop rcx
          pop rax"
        :
        :
        :
        : "intel"
        );
}

macro wrap_handler($name: path) {
    {
        #[naked]
        extern "C" fn wrapper() -> ! {
            unsafe {
                /*
                 * To calculate the address of the exception stack frame, we add 0x48 bytes (9
                 * 64-bit registers). We don't need to manually align the stack, as it should
                 * already be aligned correctly.
                 */
                save_regs!();
                asm!("mov rdi, rsp
                      add rdi, 0x48
                      call $0"
                    :
                    : "i"($name as extern "C" fn(&InterruptStackFrame))
                    : "rdi"
                    : "intel"
                    );
                restore_regs!();
                asm!("iretq"
                     :
                     :
                     :
                     : "intel"
                     );
                core::intrinsics::unreachable();
            }
        }

        wrapper
    }
}

macro wrap_handler_with_error_code($name: path) {
    {
        #[naked]
        extern "C" fn wrapper() -> ! {
            unsafe {
                /*
                 * To calculate the address of the exception stack frame, we add 0x48 bytes (9
                 * 64-bit registers), and then the two bytes of the error code. Because we skip
                 * 0x50 bytes, we need to manually align the stack.
                 */
                save_regs!();
                asm!("mov rsi, [rsp+0x48]   // Put the error code in RSI
                      mov rdi, rsp
                      add rdi, 0x50
                      sub rsp, 8            // Align the stack pointer
                      call $0
                      add rsp, 8            // Restore the stack pointer"
                     :
                     : "i"($name as extern "C" fn(&InterruptStackFrame, _error_code: u64))
                     : "rdi", "rsi"
                     : "intel"
                    );
                restore_regs!();
                asm!("add rsp, 8            // Pop the error code
                      iretq"
                     :
                     :
                     :
                     : "intel"
                    );
                core::intrinsics::unreachable();
            }
        }

        wrapper
    }
}