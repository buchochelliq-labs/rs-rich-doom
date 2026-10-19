/*
 * rs-rich-doom: force-included (after csrc/redirect.h) before each file in
 * opl/, so Chocolate Doom's music code builds against doomgeneric's
 * headers and without SDL. It only supplies names doomgeneric lacks.
 *
 * Copyright (C) 2026 rs-rich-doom contributors. GPL-2.0-or-later.
 */
#ifndef RS_RICH_DOOM_OPL_COMPAT_H
#define RS_RICH_DOOM_OPL_COMPAT_H

#include <stdint.h>
#include <stdlib.h>

/* Chocolate Doom's doomtype.h. */
#if defined(_MSC_VER)
#define PACKED_STRUCT(...) \
    __pragma(pack(push, 1)) struct __VA_ARGS__ __pragma(pack(pop))
#else
#define PACKED_STRUCT(...) struct __attribute__((packed)) __VA_ARGS__
#endif

/* SDL's big-endian loads, for midifile.c: the bytes as stored, read as
 * big-endian whatever the host's order. */
static inline uint16_t SDL_SwapBE16(uint16_t x)
{
    const uint8_t *b = (const uint8_t *)&x;
    return (uint16_t)((b[0] << 8) | b[1]);
}

static inline uint32_t SDL_SwapBE32(uint32_t x)
{
    const uint8_t *b = (const uint8_t *)&x;
    return ((uint32_t)b[0] << 24) | ((uint32_t)b[1] << 16) |
           ((uint32_t)b[2] << 8) | (uint32_t)b[3];
}

/* Chocolate Doom's i_system.c. */
void I_Error(char *error, ...);

static inline void *I_Realloc(void *ptr, size_t size)
{
    void *new_ptr = realloc(ptr, size);
    if (size != 0 && new_ptr == NULL) {
        I_Error("I_Realloc: failed on reallocation of %u bytes",
                (unsigned int)size);
    }
    return new_ptr;
}

/* Chocolate Doom's i_sound.h: the DMX version the OPL code emulates. */
typedef enum {
    opl_doom1_1_666, /* Doom 1 v1.666 */
    opl_doom2_1_666, /* Doom 2 v1.666, Hexen, Heretic */
    opl_doom_1_9     /* Doom v1.9, Strife */
} opl_driver_ver_t;

void I_SetOPLDriverVer(opl_driver_ver_t ver);

#endif
