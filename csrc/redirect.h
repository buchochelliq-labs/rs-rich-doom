/*
 * rs-rich-doom: force-included before every doomgeneric source file
 * (`-include redirect.h`), so the engine's console output and its calls to
 * exit() go to the Rust side instead of the terminal the game is drawn on.
 *
 * The C library's headers are included first, so their declarations keep
 * their own names; only the engine's call sites are renamed.
 *
 * Copyright (C) 2026 rs-rich-doom contributors. GPL-2.0-or-later.
 */
#ifndef RS_RICH_DOOM_REDIRECT_H
#define RS_RICH_DOOM_REDIRECT_H

#include <stdio.h>
#include <stdlib.h>
#include <stdarg.h>

int DG_printf(const char *format, ...);
int DG_vprintf(const char *format, va_list args);
int DG_fprintf(FILE *stream, const char *format, ...);
int DG_vfprintf(FILE *stream, const char *format, va_list args);
int DG_puts(const char *text);
int DG_putchar(int c);
#if defined(__GNUC__) || defined(__clang__)
__attribute__((noreturn))
#endif
void DG_exit(int code);

#define printf DG_printf
#define vprintf DG_vprintf
#define fprintf DG_fprintf
#define vfprintf DG_vfprintf
#define puts DG_puts
#define putchar DG_putchar
#define exit DG_exit

#endif
