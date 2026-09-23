ENTRY(reset_handler)

MEMORY {
  FLASH  (rx)    : ORIGIN = FIRMWARE_START, LENGTH = FIRMWARE_MAXSIZE
  AUX1_RAM (rw)  : ORIGIN = AUX1_RAM_START, LENGTH = AUX1_RAM_SIZE
  AUX2_RAM (rw)  : ORIGIN = AUX2_RAM_START, LENGTH = AUX2_RAM_SIZE
}

_stack_section_start = ADDR(.stack);
_stack_section_end = ADDR(.stack) + SIZEOF(.stack);
_stack_section_size = SIZEOF(.stack);

_data_section_loadaddr = LOADADDR(.data);
_data_section_start = ADDR(.data);
_data_section_end = ADDR(.data) + SIZEOF(.data);

_bss_section_start = ADDR(.bss);
_bss_section_end = ADDR(.bss) + SIZEOF(.bss);

_tls_section_start = ADDR(.tls);
_tls_section_size = SIZEOF(.tls);

_codelen = SIZEOF(.flash) + SIZEOF(.data);

_heap_start = ADDR(.heap);
_heap_end = ADDR(.heap) + SIZEOF(.heap);

SECTIONS {
  .vendorheader : ALIGN(4) {
    KEEP(*(.vendorheader))
  } >FLASH AT>FLASH

  .header : ALIGN(4) {
    KEEP(*(.header));
    . = ALIGN(CODE_ALIGNMENT);
  } >FLASH AT>FLASH

  .flash : ALIGN(CODE_ALIGNMENT) {
    KEEP(*(.kernel));
    . = ALIGN(COREAPP_ALIGNMENT);
    KEEP(*(.coreapp_header));
    . = ALIGN(4);
    *(.text*);
    . = ALIGN(4);
    *(.rodata*);
    . = ALIGN(4);
    KEEP(*(.bootloader));
    *(.bootloader*);
    . = ALIGN(512);
  } >FLASH AT>FLASH

  .data : ALIGN(4) {
    *(.data*);
    . = ALIGN(512);
  } >AUX1_RAM AT>FLASH

  /DISCARD/ : {
    *(.ARM.exidx*);
  }

  .tls : ALIGN(32) {
    *(.tls*); /* 32-byte alignment required by MPU */
    . = ALIGN(32);
  } > AUX1_RAM

  .bss : ALIGN(4) {
    *(.bss*);
    . = ALIGN(4);
  } >AUX1_RAM

  .no_dma_buffers (NOLOAD): ALIGN(4) {
    *(.no_dma_buffers*);
    . = ALIGN(4);
  } >AUX1_RAM

  /* The Zcash signing allocator's boot-rooted arena, empty unless the image
     was built with Ironwood. It is in AUX1 with .bss rather than in .buf so
     that it does not come out of the MicroPython GC heap, which .buf fills
     AUX2 ahead of. 16-byte aligned: the arena's free list is on a 16-byte
     grid. */
  .zcash_region (NOLOAD): ALIGN(16) {
    *(.zcash_region*);
    . = ALIGN(4);
  } >AUX1_RAM

  .stack : ALIGN(8) {
    . += 32K; /* Overflow causes UsageFault */
  } >AUX2_RAM

  .buf (NOLOAD): ALIGN(4) {
    *(.buf*);
    . = ALIGN(4);
  } >AUX2_RAM

  .heap : ALIGN(4) {
    . += 37K; /* this acts as a build time assertion that at least this much memory is available for heap use */
    . = ABSOLUTE(ORIGIN(AUX2_RAM) + LENGTH(AUX2_RAM)); /* this explicitly sets the end of the heap */
  } >AUX2_RAM
}
