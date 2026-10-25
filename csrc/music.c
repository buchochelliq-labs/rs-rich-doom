/*
 * rs-rich-doom: music as on DOS, OPL FM synthesis. The engine's music
 * module (DG_music_module) is Chocolate Doom's OPL module (opl/i_oplmusic.c:
 * MUS to MIDI with mus2mid, the GENMIDI lump's instrument patches, DMX's
 * voice allocation) driving the Nuked OPL3 emulator (opl/opl3.c) through
 * the software OPL driver here, which takes the place of Chocolate Doom's
 * SDL driver (opl/opl_sdl.c, which this follows). The Rust mixer
 * (src/sound.rs) pulls the samples with rs_doom_music_render().
 *
 * Threads: the engine thread calls the module (play, stop, volume, pause)
 * and the audio thread renders, which runs the song's timer callbacks.
 * One mutex covers all of it: every module entry point and every render
 * holds it, so the OPL chip, the callback queue and the song state are only
 * touched by one thread at a time. OPL_Lock and OPL_Unlock, which the
 * module calls inside its entry points, are no-ops for that reason (the
 * lock is already held).
 *
 * Copyright (C) 2026 rs-rich-doom contributors. GPL-2.0-or-later.
 * The driver follows Chocolate Doom's opl/opl_sdl.c, Copyright (C)
 * 2005-2014 Simon Howard (GPL-2.0-or-later).
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "doomtype.h"
#include "i_sound.h"
#include "memio.h"
#include "mus2mid.h"
#include "z_zone.h"

#include "opl.h"
#include "opl3.h"
#include "opl_internal.h"
#include "opl_queue.h"

/* Implemented in Rust (src/sound.rs): the rate to synthesize at, the
 * output device's (opening it if need be), or 0 for no music. */
extern unsigned int rs_doom_music_rate(void);

/* opl/i_oplmusic.c */
extern music_module_t music_opl_module;
extern const byte *rs_doom_genmidi;

/* ---- The lock ---- */

#if defined(_WIN32)
#include <windows.h>
static SRWLOCK music_lock = SRWLOCK_INIT;
static void Lock(void) { AcquireSRWLockExclusive(&music_lock); }
static void Unlock(void) { ReleaseSRWLockExclusive(&music_lock); }
#else
#include <pthread.h>
static pthread_mutex_t music_lock = PTHREAD_MUTEX_INITIALIZER;
static void Lock(void) { pthread_mutex_lock(&music_lock); }
static void Unlock(void) { pthread_mutex_unlock(&music_lock); }
#endif

/* ---- The software OPL driver (after opl_sdl.c) ---- */

typedef struct {
    unsigned int rate;    /* Number of times the timer is advanced per sec. */
    unsigned int enabled; /* Non-zero if timer is enabled. */
    unsigned int value;   /* Last value that was set. */
    uint64_t expire_time; /* Calculated time that timer will expire. */
} opl_timer_t;

static int driver_ready;
static unsigned int mixing_freq;
static opl_callback_queue_t *callback_queue;
/* Current time, in us since the driver started. */
static uint64_t current_time;
static int opl_paused;
/* Time (in us) the callbacks were paused for. */
static uint64_t pause_offset;
static opl3_chip opl_chip;
static int register_num;
static opl_timer_t timer1 = {12500, 0, 0, 0};
static opl_timer_t timer2 = {3125, 0, 0, 0};

/* Advance time by `us`, invoking the callbacks that come due. */
static void AdvanceTimeUs(uint64_t us)
{
    opl_callback_t callback;
    void *callback_data;

    current_time += us;
    if (opl_paused) {
        pause_offset += us;
    }
    while (!OPL_Queue_IsEmpty(callback_queue) &&
           current_time >= OPL_Queue_Peek(callback_queue) + pause_offset) {
        if (!OPL_Queue_Pop(callback_queue, &callback, &callback_data)) {
            break;
        }
        /* The lock is held (by the render): a callback may schedule more. */
        callback(callback_data);
    }
}

void OPL_RS_Delay(uint64_t us) { AdvanceTimeUs(us); }

static int RS_Init(unsigned int port_base)
{
    (void)port_base;
    mixing_freq = rs_doom_music_rate();
    if (mixing_freq == 0) {
        return 0;
    }
    callback_queue = OPL_Queue_Create();
    current_time = 0;
    opl_paused = 0;
    pause_offset = 0;
    register_num = 0;
    memset(&timer1, 0, sizeof timer1);
    memset(&timer2, 0, sizeof timer2);
    timer1.rate = 12500;
    timer2.rate = 3125;
    OPL3_Reset(&opl_chip, mixing_freq);
    driver_ready = 1;
    return 1;
}

static void RS_Shutdown(void)
{
    driver_ready = 0;
    if (callback_queue != NULL) {
        OPL_Queue_Destroy(callback_queue);
        callback_queue = NULL;
    }
}

static unsigned int RS_PortRead(opl_port_t port)
{
    unsigned int result = 0;

    if (port == OPL_REGISTER_PORT_OPL3) {
        return 0xff;
    }
    if (timer1.enabled && current_time > timer1.expire_time) {
        result |= 0x80; /* Either have expired */
        result |= 0x40; /* Timer 1 has expired */
    }
    if (timer2.enabled && current_time > timer2.expire_time) {
        result |= 0x80; /* Either have expired */
        result |= 0x20; /* Timer 2 has expired */
    }
    return result;
}

static void TimerEndTime(opl_timer_t *timer)
{
    if (timer->enabled) {
        int tics = 0x100 - timer->value;
        timer->expire_time =
            current_time + ((uint64_t)tics * OPL_SECOND) / timer->rate;
    }
}

static void WriteRegister(unsigned int reg_num, unsigned int value)
{
    switch (reg_num) {
    case OPL_REG_TIMER1:
        timer1.value = value;
        TimerEndTime(&timer1);
        break;
    case OPL_REG_TIMER2:
        timer2.value = value;
        TimerEndTime(&timer2);
        break;
    case OPL_REG_TIMER_CTRL:
        if (value & 0x80) {
            timer1.enabled = 0;
            timer2.enabled = 0;
        } else {
            if ((value & 0x40) == 0) {
                timer1.enabled = (value & 0x01) != 0;
                TimerEndTime(&timer1);
            }
            if ((value & 0x20) == 0) {
                timer2.enabled = (value & 0x02) != 0;
                TimerEndTime(&timer2);
            }
        }
        break;
    default:
        OPL3_WriteRegBuffered(&opl_chip, (uint16_t)reg_num, (uint8_t)value);
        break;
    }
}

static void RS_PortWrite(opl_port_t port, unsigned int value)
{
    if (port == OPL_REGISTER_PORT) {
        register_num = value;
    } else if (port == OPL_REGISTER_PORT_OPL3) {
        register_num = value | 0x100;
    } else if (port == OPL_DATA_PORT) {
        WriteRegister(register_num, value);
    }
}

static void RS_SetCallback(uint64_t us, opl_callback_t callback, void *data)
{
    OPL_Queue_Push(callback_queue, callback, data,
                   current_time - pause_offset + us);
}

static void RS_ClearCallbacks(void) { OPL_Queue_Clear(callback_queue); }

/* The module's critical sections already hold the lock. */
static void RS_Lock(void) {}
static void RS_Unlock(void) {}

static void RS_SetPaused(int paused) { opl_paused = paused; }

static void RS_AdjustCallbacks(float factor)
{
    OPL_Queue_AdjustCallbacks(callback_queue, current_time, factor);
}

opl_driver_t opl_rs_driver = {
    "rs-rich-doom",    RS_Init,         RS_Shutdown,       RS_PortRead,
    RS_PortWrite,      RS_SetCallback,  RS_ClearCallbacks, RS_Lock,
    RS_Unlock,         RS_SetPaused,    RS_AdjustCallbacks,
};

/*
 * Fill `out` with `frames` interleaved stereo frames at the rate
 * rs_doom_music_rate() gave, running the song's callbacks as their time
 * comes. Silence when no music is set up. Returns 1 when the chip ran.
 */
int rs_doom_music_render(int16_t *out, unsigned int frames)
{
    unsigned int filled = 0;

    Lock();
    if (!driver_ready || out == NULL) {
        Unlock();
        if (out != NULL) {
            memset(out, 0, (size_t)frames * 2 * sizeof(int16_t));
        }
        return 0;
    }
    while (filled < frames) {
        uint64_t nsamples = frames - filled;

        /* Up to the next callback, so it runs at its sample. */
        if (!opl_paused && !OPL_Queue_IsEmpty(callback_queue)) {
            uint64_t next = OPL_Queue_Peek(callback_queue) + pause_offset;
            uint64_t until = next > current_time ? next - current_time : 0;
            uint64_t n = (until * mixing_freq + OPL_SECOND - 1) / OPL_SECOND;
            if (n < nsamples) {
                nsamples = n;
            }
        }
        if (nsamples > 0) {
            OPL3_GenerateStream(&opl_chip, out + (size_t)filled * 2,
                                (uint32_t)nsamples);
        }
        filled += (unsigned int)nsamples;
        AdvanceTimeUs((nsamples * OPL_SECOND) / mixing_freq);
    }
    Unlock();
    return 1;
}

/* ---- The engine's music module: the OPL module, under the lock ---- */

static boolean MusicInit(void)
{
    boolean ok;
    Lock();
    ok = music_opl_module.Init();
    Unlock();
    return ok;
}

static void MusicShutdown(void)
{
    Lock();
    music_opl_module.Shutdown();
    Unlock();
}

static void SetMusicVolume(int volume)
{
    Lock();
    music_opl_module.SetMusicVolume(volume);
    Unlock();
}

static void PauseMusic(void)
{
    Lock();
    music_opl_module.PauseMusic();
    Unlock();
}

static void ResumeMusic(void)
{
    Lock();
    music_opl_module.ResumeMusic();
    Unlock();
}

static void *RegisterSong(void *data, int len)
{
    void *handle;
    Lock();
    handle = music_opl_module.RegisterSong(data, len);
    Unlock();
    return handle;
}

static void UnRegisterSong(void *handle)
{
    Lock();
    music_opl_module.UnRegisterSong(handle);
    Unlock();
}

static void PlaySong(void *handle, boolean looping)
{
    Lock();
    music_opl_module.PlaySong(handle, looping);
    Unlock();
}

static void StopSong(void)
{
    Lock();
    music_opl_module.StopSong();
    Unlock();
}

static boolean MusicIsPlaying(void)
{
    boolean playing;
    Lock();
    playing = music_opl_module.MusicIsPlaying();
    Unlock();
    return playing;
}

static void Poll(void) {}

static snddevice_t music_devices[] = {SNDDEVICE_ADLIB, SNDDEVICE_SB};

music_module_t DG_music_module = {
    music_devices,  sizeof music_devices / sizeof music_devices[0],
    MusicInit,      MusicShutdown,
    SetMusicVolume, PauseMusic,
    ResumeMusic,    RegisterSong,
    UnRegisterSong, PlaySong,
    StopSong,       MusicIsPlaying,
    Poll,
};

/* ---- Without the engine, for tests (src/sound.rs) ---- */

static void *test_song;

/* memio, which mus2mid writes to, allocates from the engine's zone. */
static void TestZone(void)
{
    static int zone_ready;
    if (!zone_ready) {
        Z_Init();
        zone_ready = 1;
    }
}

/* Start the module with this GENMIDI lump instead of the WAD's, and play
 * `song` (MUS or MIDI). Only when the engine is not running. */
int rs_doom_music_test_start(const uint8_t *genmidi, const uint8_t *song,
                             int len, int volume)
{
    TestZone();
    rs_doom_genmidi = genmidi;
    if (!MusicInit()) {
        rs_doom_genmidi = NULL;
        return 0;
    }
    SetMusicVolume(volume);
    test_song = RegisterSong((void *)song, len);
    if (test_song == NULL) {
        MusicShutdown();
        rs_doom_genmidi = NULL;
        return 0;
    }
    PlaySong(test_song, true);
    return 1;
}

void rs_doom_music_test_stop(void)
{
    StopSong();
    UnRegisterSong(test_song);
    test_song = NULL;
    MusicShutdown();
    rs_doom_genmidi = NULL;
}

/* MUS to MIDI with the engine's mus2mid, into `out` (at most `cap`
 * bytes). Returns the MIDI's length, or -1. */
long rs_doom_mus2mid(const uint8_t *mus, size_t len, uint8_t *out,
                     size_t cap)
{
    MEMFILE *in;
    MEMFILE *mid;
    long result = -1;

    TestZone();
    in = mem_fopen_read((void *)mus, len);
    mid = mem_fopen_write();

    if (!mus2mid(in, mid)) {
        void *buf;
        size_t buflen;
        mem_get_buf(mid, &buf, &buflen);
        if (buflen <= cap) {
            memcpy(out, buf, buflen);
            result = (long)buflen;
        }
    }
    mem_fclose(in);
    mem_fclose(mid);
    return result;
}
