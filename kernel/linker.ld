/* OpenSBI (QEMU's default firmware) jumps to the kernel at 0x80200000 in
   supervisor mode with a0 = hart id and a1 = device tree address. */
OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    . = 0x80200000;
    __kernel_start = .;

    .text : {
        *(.text.entry)
        *(.text .text.*)
    }

    .rodata : ALIGN(8) {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }

    .data : ALIGN(8) {
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
