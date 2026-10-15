/* OpenSBI (QEMU's default firmware) jumps to the kernel at 0x80200000 in
   supervisor mode with a0 = hart id and a1 = device tree address. */
OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    . = 0x80200000;
    __kernel_start = .;

    /* Sections are page-aligned so each can get its own permissions. */
    .text : ALIGN(4096) {
        __text_start = .;
        *(.text.entry)
        *(.text .text.*)
        . = ALIGN(4096);
        __text_end = .;
    }

    .rodata : ALIGN(4096) {
        __rodata_start = .;
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
        . = ALIGN(4096);
        __rodata_end = .;
    }

    .data : ALIGN(4096) {
        __data_start = .;
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    .bss (NOLOAD) : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        . = ALIGN(8);
        __bss_end = .;
    }

    /* 64 KiB boot stack */
    . = ALIGN(4096);
    __stack_bottom = .;
    . += 0x10000;
    __stack_top = .;

    __kernel_end = .;

    /DISCARD/ : { *(.eh_frame) *(.eh_frame_hdr) }
}
