/*
 * rs-rich-doom: doomgeneric's sound module (DG_sound_module), mixing in
 * Rust (src/sound.rs, played through intuiTUIve's audio output). The engine
 * says which effect to start on which channel at what volume and stereo
 * separation; this reads the effect's DMX lump (8-bit unsigned PCM,
 * usually 11025 Hz) and hands the samples to Rust, which resamples and
 * mixes them.
 *
 * Music is csrc/music.c: OPL FM synthesis, mixed in by the same Rust code.
 *
 * Copyright (C) 2026 rs-rich-doom contributors. GPL-2.0-or-later.
 * The lump parsing follows Chocolate Doom's i_sdlsound.c (GPL-2.0-or-later).
 */
#include <stdint.h>
#include <stdio.h>

#include "doomtype.h"
#include "deh_str.h"
#include "i_sound.h"
#include "m_misc.h"
#include "w_wad.h"
#include "z_zone.h"

/* Implemented in Rust (src/sound.rs). */
extern int rs_doom_sound_init(void);
extern void rs_doom_sound_shutdown(void);
extern void rs_doom_sound_start(int channel, int lump, const uint8_t *samples,
                                size_t len, int rate, int vol, int sep);
extern void rs_doom_sound_params(int channel, int vol, int sep);
extern void rs_doom_sound_stop(int channel);
extern int rs_doom_sound_playing(int channel);

/* Config variables the engine binds when it is built with sound. */
int use_libsamplerate = 0;
float libsamplerate_scale = 0.65f;

static boolean use_prefix;

static boolean Init(boolean use_sfx_prefix)
{
    use_prefix = use_sfx_prefix;
    return rs_doom_sound_init() ? true : false;
}

static void Shutdown(void) { rs_doom_sound_shutdown(); }

static int GetSfxLumpNum(sfxinfo_t *sfx)
{
    char name[9];
    if (sfx->link != NULL) {
        sfx = sfx->link;
    }
    if (use_prefix) {
        M_snprintf(name, sizeof name, "ds%s", DEH_String(sfx->name));
    } else {
        M_StringCopy(name, DEH_String(sfx->name), sizeof name);
    }
    return W_GetNumForName(name);
}

static void Update(void) {}

static void UpdateSoundParams(int channel, int vol, int sep)
{
    rs_doom_sound_params(channel, vol, sep);
}

static int StartSound(sfxinfo_t *sfx, int channel, int vol, int sep)
{
    int lump = sfx->lumpnum;
    unsigned int lumplen = W_LumpLength(lump);
    const uint8_t *data = W_CacheLumpNum(lump, PU_STATIC);

    /* Format 3: a 16-bit sample rate, a 32-bit length, then the samples. */
    if (lumplen < 8 || data[0] != 0x03 || data[1] != 0x00) {
        return -1;
    }
    int rate = (data[3] << 8) | data[2];
    unsigned int length = ((unsigned int)data[7] << 24) | (data[6] << 16) |
                          (data[5] << 8) | data[4];
    if (length > lumplen - 8 || length <= 48) {
        return -1;
    }
    /* DMX skips the first and last 16 bytes of the samples. */
    rs_doom_sound_start(channel, lump, data + 8 + 16, length - 32, rate, vol,
                        sep);
    return channel;
}

static void StopSound(int channel) { rs_doom_sound_stop(channel); }

static boolean SoundIsPlaying(int channel)
{
    return rs_doom_sound_playing(channel) ? true : false;
}

static void CacheSounds(sfxinfo_t *sounds, int num_sounds)
{
    (void)sounds;
    (void)num_sounds;
}

static snddevice_t devices[] = {
    SNDDEVICE_SB,          SNDDEVICE_PAS,         SNDDEVICE_GUS,
    SNDDEVICE_WAVEBLASTER, SNDDEVICE_SOUNDCANVAS, SNDDEVICE_AWE32,
};

sound_module_t DG_sound_module = {
    devices,
    sizeof devices / sizeof devices[0],
    Init,
    Shutdown,
    GetSfxLumpNum,
    Update,
    UpdateSoundParams,
    StartSound,
    StopSound,
    SoundIsPlaying,
    CacheSounds,
};

/* Music is csrc/music.c (DG_music_module). */

/* ---- ALSA's own error messages would land on the game's screen. ---- */

#if defined(__linux__)
#include <alsa/asoundlib.h>

static void quiet(const char *file, int line, const char *function, int err,
                  const char *format, ...)
{
    (void)file;
    (void)line;
    (void)function;
    (void)err;
    (void)format;
}

void rs_doom_quiet_alsa(void) { snd_lib_error_set_handler(quiet); }
#else
void rs_doom_quiet_alsa(void) {}
#endif
