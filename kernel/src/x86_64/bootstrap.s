; Copyright (C) 2017, Isaac Woods. 
; See LICENCE.md

; This is physically-mapped into the executable, so we don't have to faff about with PIC. It starts
; in 32-bit PM, identity-maps the first couple of MBs and enters long-mode, then sets up some new
; page-tables to map the kernel into the higher-half, then jumps into that code.

section .multiboot
multiboot_header:
  dd 0xe85250d6                                                         ; Multiboot-2 magic
  dd 0                                                                  ; Architecture=0 (P-mode i386)
  dd multiboot_end - multiboot_header                                   ; Header length
  dd 0x100000000-(0xe85250d6 + 0 + (multiboot_end - multiboot_header))  ; Checksum

  ; More options can be inserted here

  dw 0
  dw 0
  dd 8
multiboot_end:

section .text
bits 32

; Prints "ERR: " followed by the ASCII character in AL. The last thing on the stack should be the address that called this function.
;   'M' = Incorrect Multiboot magic
;   'C' = CPUID instruction is not supported
;   'L' = Long mode not available
PrintError:
  mov dword [0xb8000], 0x4f524f45
  mov dword [0xb8004], 0x4f3a4f52
  mov dword [0xb8008], 0x4f204f20
  mov byte  [0xb800a], al
  hlt

CheckCpuidSupported:
  pushfd          ; Copy EFLAGS into EAX
  pop eax
  mov ecx, eax    ; Make a copy in ECX to compare later on
  xor eax, 1<<21  ; Flip the ID bit
  push eax        ; Copy EAX back into EFLAGS
  popfd
  pushfd          ; Read EFLAGS again (with the ID bit flipped or not)
  pop eax
  push ecx        ; Restore EFLAGS back to the old version
  popfd

  ; Compare the (potentially) flipped version to the first one
  cmp eax, ecx
  je .no_cpuid
  ret
.no_cpuid:
  mov al, 'C'
  call PrintError

CheckLongModeSupported:
  ; Test if we can access the Extended Processor Info
  mov eax, 0x80000000
  cpuid
  cmp eax, 0x80000001
  jb .no_long_mode

  ; Check the EPI to see if long mode is available on this CPU
  mov eax, 0x80000001
  cpuid
  test edx, 1<<29
  jz .no_long_mode
  ret
.no_long_mode:
  mov al, 'L'
  call PrintError

SetupIdentityMap:
  ; Map first entry of P4 to P3
  mov eax, identity_p3
  or eax, 0b11  ; Present + Writable
  mov [identity_p4], eax

  ; Map first entry of P3 to P2
  mov eax, identity_p2
  or eax, 0b11    ; Present + Writable
  mov [identity_p3], eax

  ; Map every P2 entry to a huge page starting at ECX * 2MiB
  mov ecx, 0
.map_p2_entry:
  mov eax, 0x200000
  mul ecx
  or eax, 0b10000011  ; Present + Writable + Huge
  mov [identity_p2 + ecx * 8], eax

  inc ecx
  cmp ecx, 512
  jne .map_p2_entry

  ret

EnablePaging:
  ; Load the P4 pointer into CR3
  mov eax, identity_p4
  mov cr3, eax

  ; Enable PAE
  mov eax, cr4
  or eax, 1 << 5
  mov cr4, eax

  ; Enable Long-Mode in the EFER MSR
  mov ecx, 0xC0000080
  rdmsr
  or eax, 1 << 8
  wrmsr

  ; Enable paging
  mov eax, cr0
  or eax, 1 << 31
  mov cr0, eax

  ret

global BootstrapStart
BootstrapStart:
  mov esp, bootstrap_stack_top
  mov edi, ebx  ; Move the pointer to the Multiboot structure into EDI

  ; Check that GRUB passed us the correct magic number
  cmp eax, 0x36d76289
  je .multiboot_fine
  mov al, 'M'
  call PrintError
.multiboot_fine:

  call CheckCpuidSupported
  call CheckLongModeSupported

  call SetupIdentityMap
  call EnablePaging

  ; We're now technically in Long-Mode, but we've been put in 32-bit compatibility submode until we
  ; install a valid GDT. We can then far-jump into the new code segment (in real Long-Mode :P).
  lgdt [gdt64.pointer]
  jmp gdt64.codeSegment:InLongMode

bits 64
InLongMode:
  ; Reload every data segment with 0 (Long-mode doesn't use segmentation)
  mov ax, 0
  mov ss, ax
  mov ds, ax
  mov es, ax
  mov fs, ax
  mov gs, ax

  ; print OKAY
  mov rax, 0x2f592f412f4b2f4f
  mov qword [0xb8000], rax
  hlt

section .bss
align 4096
identity_p4:
  resb 4096
identity_p3:
  resb 4096
identity_p2:
  resb 4096
bootstrap_stack_bottom:
  resb 4096
bootstrap_stack_top:

section .rodata
gdt64:
  .nullEntry:  equ $-gdt64
    dq 0
  .codeSegment: equ $-gdt64
    dq (1<<43)|(1<<44)|(1<<47)|(1<<53)
  .pointer:
    dw $-gdt64-1
    dq gdt64