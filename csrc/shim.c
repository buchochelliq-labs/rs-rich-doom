/*
 * rs-rich-doom: the doomgeneric platform layer, implemented by calling back
 * into Rust (src/engine.rs). doomgeneric asks a port for six functions
 * (DG_Init, DG_DrawFrame, DG_SleepMs, DG_GetTicksMs, DG_GetKey,
 * DG_SetWindowTitle); each is a thin forward here.
 *
 * Also here: the console-output and exit() replacements that redirect.h
 * renames the engine's calls to.
 *
 * Copyright (C) 2026 rs-rich-doom contributors. GPL-2.0-or-later.
 */
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "doomgeneric.h"

/* Implemented in Rust (src/engine.rs). */
extern void rs_doom_init(void);
extern void rs_doom_draw_frame(const uint32_t *pixels, int width, int height);
extern void rs_doom_sleep_ms(uint32_t ms);
extern uint32_t rs_doom_ticks_ms(void);
extern int rs_doom_get_key(int *pressed, unsigned char *key);
extern void rs_doom_set_title(const char *title);
extern void rs_doom_log(const char *text, size_t len);
#if defined(__GNUC__) || defined(__clang__)
__attribute__((noreturn))
#endif
extern void rs_doom_exit(int code);

void DG_Init(void) { rs_doom_init(); }

void DG_DrawFrame(void)
{
    rs_doom_draw_frame((const uint32_t *)DG_ScreenBuffer, DOOMGENERIC_RESX,
                       DOOMGENERIC_RESY);
}

void DG_SleepMs(uint32_t ms) { rs_doom_sleep_ms(ms); }

uint32_t DG_GetTicksMs(void) { return rs_doom_ticks_ms(); }

int DG_GetKey(int *pressed, unsigned char *key)
{
    return rs_doom_get_key(pressed, key);
}

void DG_SetWindowTitle(const char *title) { rs_doom_set_title(title); }

/* The width and height the engine was compiled for. */
int rs_doom_resx(void) { return DOOMGENERIC_RESX; }
int rs_doom_resy(void) { return DOOMGENERIC_RESY; }

/* ---- Console output: to the Rust side's log, never the terminal. ---- */

int DG_vprintf(const char *format, va_list args)
{
    char buffer[2048];
    int n = vsnprintf(buffer, sizeof buffer, format, args);
    if (n < 0) {
        return n;
    }
    size_t len = (size_t)n < sizeof buffer ? (size_t)n : sizeof buffer - 1;
    rs_doom_log(buffer, len);
    return n;
}

int DG_printf(const char *format, ...)
{
    va_list args;
    va_start(args, format);
    int n = DG_vprintf(format, args);
    va_end(args);
    return n;
}

int DG_vfprintf(FILE *stream, const char *format, va_list args)
{
    if (stream == stdout || stream == stderr) {
        return DG_vprintf(format, args);
    }
    return vfprintf(stream, format, args);
}

int DG_fprintf(FILE *stream, const char *format, ...)
{
    va_list args;
    va_start(args, format);
    int n = DG_vfprintf(stream, format, args);
    va_end(args);
    return n;
}

int DG_puts(const char *text)
{
    rs_doom_log(text, strlen(text));
    rs_doom_log("\n", 1);
    return 1;
}

int DG_putchar(int c)
{
    char ch = (char)c;
    rs_doom_log(&ch, 1);
    return c;
}

void DG_exit(int code) { rs_doom_exit(code); }
