/* Our own shim (not aoTuV code): the oracle's entry points for Rust (crates/aotuv-sys): an
   Ogg Vorbis encoder as libvorbis' example encoder drives it (vorbis_encode_init_vbr or _init (managed),
   headers flushed to their own pages, each packet paged out with ogg_stream_pageout, a final
   flush), and the kernels behind opaque handles. Compiled once per copy: SHIM(x) is aotuv_x
   (the strict oracle), lancer_x (-DSHIM_LANCER) or xiph_x (-DSHIM_XIPH: xiph's libvorbis):
   the speed baselines, encoder only, the rest made local. */
#include <stdlib.h>
#include <string.h>
#include <vorbis/vorbisenc.h>
#include "mdct.h"
#include "smallft.h"
#include "codec_internal.h"
#include "psy.h"
#include "envelope.h"
#include "masking.h"

#ifdef SHIM_LANCER
#define SHIM(x) lancer_##x
#define SHIM_SPEED
#elif defined(SHIM_XIPH)
#define SHIM(x) xiph_##x
#define SHIM_SPEED
#else
#define SHIM(x) aotuv_##x
#endif

typedef struct {
  vorbis_info vi; vorbis_dsp_state vd; vorbis_block vb; vorbis_comment vc;
  ogg_stream_state os;
  unsigned char *out; long len, cap;
  /* every packet as handed to libogg: [i64 bytes][i64 granulepos][u8 e_o_s][bytes] */
  unsigned char *pk; long pklen, pkcap;
  /* per block, every packet blob as mapping0 left it (before bitrate management):
     PACKETBLOBS x [i64 bytes][bytes] */
  unsigned char *bl; long bllen, blcap;
} enc;

static void put(enc *e, const unsigned char *p, long n) {
  if (e->len + n > e->cap) {
    e->cap = (e->len + n) * 2;
    e->out = realloc(e->out, e->cap);
  }
  memcpy(e->out + e->len, p, n);
  e->len += n;
}
static void logpacket(enc *e, ogg_packet *op) {
  long need = e->pklen + 17 + op->bytes;
  if (need > e->pkcap) {
    e->pkcap = need * 2;
    e->pk = realloc(e->pk, e->pkcap);
  }
  long long b = op->bytes, g = op->granulepos;
  memcpy(e->pk + e->pklen, &b, 8);
  memcpy(e->pk + e->pklen + 8, &g, 8);
  e->pk[e->pklen + 16] = op->e_o_s ? 1 : 0;
  memcpy(e->pk + e->pklen + 17, op->packet, op->bytes);
  e->pklen = need;
}
static void logblobs(enc *e) {
  vorbis_block_internal *vbi = e->vb.internal;
  for (int k = 0; k < PACKETBLOBS; k++) {
    long long b = oggpack_bytes(vbi->packetblob[k]);
    long need = e->bllen + 8 + b;
    if (need > e->blcap) {
      e->blcap = need * 2;
      e->bl = realloc(e->bl, e->blcap);
    }
    memcpy(e->bl + e->bllen, &b, 8);
    if (b) memcpy(e->bl + e->bllen + 8, oggpack_get_buffer(vbi->packetblob[k]), b);
    e->bllen = need;
  }
}
static void page(enc *e, ogg_page *og) {
  put(e, og->header, og->header_len);
  put(e, og->body, og->body_len);
}

/* managed == 0: VBR at quality q; else vorbis_encode_init(max, nominal=bitrate, min). */
enc *SHIM(enc_new)(int ch, long rate, int managed, float q, long min, long bitrate, long max) {
  enc *e = calloc(1, sizeof(*e));
  ogg_packet h0, h1, h2;
  ogg_page og;
  vorbis_info_init(&e->vi);
  if ((!managed ? vorbis_encode_init_vbr(&e->vi, ch, rate, q)
                   : vorbis_encode_init(&e->vi, ch, rate, max, bitrate, min))) {
    vorbis_info_clear(&e->vi);
    free(e);
    return 0;
  }
  vorbis_comment_init(&e->vc);
  vorbis_analysis_init(&e->vd, &e->vi);
  vorbis_block_init(&e->vd, &e->vb);
  ogg_stream_init(&e->os, 1);
  vorbis_analysis_headerout(&e->vd, &e->vc, &h0, &h1, &h2);
  logpacket(e, &h0);
  logpacket(e, &h1);
  logpacket(e, &h2);
  ogg_stream_packetin(&e->os, &h0);
  ogg_stream_packetin(&e->os, &h1);
  ogg_stream_packetin(&e->os, &h2);
  while (ogg_stream_flush(&e->os, &og)) page(e, &og);
  return e;
}

/* n samples per channel (planar); n == 0 ends the stream (the final pages are flushed). */
void SHIM(enc_write)(enc *e, const float *const *pcm, long n) {
  ogg_packet op;
  ogg_page og;
  if (n) {
    float **b = vorbis_analysis_buffer(&e->vd, n);
    for (int c = 0; c < e->vi.channels; c++) memcpy(b[c], pcm[c], n * sizeof(float));
  }
  vorbis_analysis_wrote(&e->vd, n);
  while (vorbis_analysis_blockout(&e->vd, &e->vb) == 1) {
    vorbis_analysis(&e->vb, 0);
    logblobs(e);
    vorbis_bitrate_addblock(&e->vb);
    while (vorbis_bitrate_flushpacket(&e->vd, &op)) {
      logpacket(e, &op);
      ogg_stream_packetin(&e->os, &op);
      while (ogg_stream_pageout(&e->os, &og)) page(e, &og);
    }
  }
  if (!n)
    while (ogg_stream_flush(&e->os, &og)) page(e, &og);
}

/* The bytes produced so far (taken: the buffer restarts empty). */
const unsigned char *SHIM(enc_out)(enc *e, long *len) {
  *len = e->len;
  e->len = 0;
  return e->out;
}

/* The packet log so far (taken, as enc_out). */
const unsigned char *SHIM(enc_packets)(enc *e, long *len) {
  *len = e->pklen;
  e->pklen = 0;
  return e->pk;
}

/* The blob log so far (taken, as enc_out). */
const unsigned char *SHIM(enc_blobs)(enc *e, long *len) {
  *len = e->bllen;
  e->bllen = 0;
  return e->bl;
}

void SHIM(enc_free)(enc *e) {
  ogg_stream_clear(&e->os);
  vorbis_block_clear(&e->vb);
  vorbis_dsp_clear(&e->vd);
  vorbis_comment_clear(&e->vc);
  vorbis_info_clear(&e->vi);
  free(e->out);
  free(e->pk);
  free(e->bl);
  free(e);
}

#ifndef SHIM_SPEED
/* The three header packets (vorbis_analysis_headerout, no user comments) of a configuration
   (managed as in enc_new), back to back in out[..cap], their lengths
   in len[3]; returns 0, the vorbis_encode_init* error (OV_EINVAL -131, OV_EIMPL -130), or -1
   when cap is too small. */
int aotuv_headers(int ch, long rate, int managed, float q, long min, long bitrate, long max,
                  unsigned char *out, long cap, long *len) {
  vorbis_info vi;
  vorbis_dsp_state vd;
  vorbis_comment vc;
  ogg_packet h[3];
  int ret;
  vorbis_info_init(&vi);
  ret = !managed ? vorbis_encode_init_vbr(&vi, ch, rate, q)
                    : vorbis_encode_init(&vi, ch, rate, max, bitrate, min);
  if (ret) return ret;
  vorbis_comment_init(&vc);
  vorbis_analysis_init(&vd, &vi);
  vorbis_analysis_headerout(&vd, &vc, &h[0], &h[1], &h[2]);
  long at = 0;
  for (int i = 0; i < 3; i++) {
    len[i] = h[i].bytes;
    if (at + h[i].bytes > cap) ret = -1;
    else memcpy(out + at, h[i].packet, h[i].bytes);
    at += h[i].bytes;
  }
  vorbis_dsp_clear(&vd);
  vorbis_comment_clear(&vc);
  vorbis_info_clear(&vi);
  return ret;
}

/* The rest of the setup the headers don't carry, flattened to doubles in struct order: psys,
   then each vorbis_info_psy, vorbis_info_psy_global, block_lowpassr, the highlevel settings
   and the bitrate manager's info. Returns the count (or the init error, or -1: cap too small). */
#define PUT(x) do { if (n >= cap) return -1; out[n++] = (double)(x); } while (0)
#define PUTS(a, len) do { for (int i_ = 0; i_ < (len); i_++) PUT((a)[i_]); } while (0)
long aotuv_setup_dump(int ch, long rate, int managed, float q, long min, long bitrate, long max,
                      double *out, long cap) {
  vorbis_info vi;
  long n = 0;
  int ret;
  vorbis_info_init(&vi);
  ret = !managed ? vorbis_encode_init_vbr(&vi, ch, rate, q)
                 : vorbis_encode_init(&vi, ch, rate, max, bitrate, min);
  if (ret) return ret;
  codec_setup_info *ci = vi.codec_setup;
  PUT(ci->psys);
  for (int b = 0; b < ci->psys; b++) {
    vorbis_info_psy *p = ci->psy_param[b];
    PUT(p->blockflag); PUT(p->ath_adjatt); PUT(p->ath_maxatt); PUTS(p->tone_masteratt, P_NOISECURVES);
    PUT(p->tone_centerboost); PUT(p->tone_decay); PUT(p->tone_abs_limit); PUTS(p->toneatt, P_BANDS);
    PUT(p->noisemaskp); PUT(p->noisemaxsupp); PUT(p->noisewindowlo); PUT(p->noisewindowhi);
    PUT(p->noisewindowlomin); PUT(p->noisewindowhimin); PUT(p->noisewindowfixed);
    for (int j = 0; j < P_NOISECURVES; j++) PUTS(p->noiseoff[j], P_BANDS);
    PUTS(p->noisecompand, NOISE_COMPAND_LEVELS); PUTS(p->noisecompand_high, NOISE_COMPAND_LEVELS);
    PUT(p->flacint); PUT(p->max_curve_dB); PUT(p->normal_p); PUT(p->normal_start);
    PUT(p->normal_partition); PUT(p->normal_thresh);
  }
  vorbis_info_psy_global *g = &ci->psy_g_param;
  PUT(g->eighth_octave_lines); PUTS(g->preecho_thresh, VE_BANDS); PUTS(g->postecho_thresh, VE_BANDS);
  PUT(g->stretch_penalty); PUT(g->preecho_minenergy); PUT(g->ampmax_att_per_sec);
  PUTS(g->coupling_pkHz, PACKETBLOBS); PUTS(g->coupling_pointlimit[0], PACKETBLOBS);
  PUTS(g->coupling_pointlimit[1], PACKETBLOBS); PUTS(g->coupling_prepointamp, PACKETBLOBS);
  PUTS(g->coupling_postpointamp, PACKETBLOBS); PUTS(g->sliding_lowpass[0], PACKETBLOBS);
  PUTS(g->sliding_lowpass[1], PACKETBLOBS);
  PUTS(ci->block_lowpassr, 2);
  highlevel_encode_setup *hi = &ci->hi;
  PUT(hi->base_setting); PUT(hi->impulse_noisetune); PUT(hi->req); PUT(hi->managed);
  PUT(hi->bitrate_min); PUT(hi->bitrate_av); PUT(hi->bitrate_av_damp); PUT(hi->bitrate_max);
  PUT(hi->bitrate_reservoir); PUT(hi->bitrate_reservoir_bias); PUT(hi->stereo_point_setting);
  PUT(hi->lowpass_kHz); PUT(hi->ath_floating_dB); PUT(hi->ath_absolute_dB);
  PUT(hi->amplitude_track_dBpersec); PUT(hi->trigger_setting); PUT(hi->pre_amplitude);
  bitrate_manager_info *bi = &ci->bi;
  PUT(bi->avg_rate); PUT(bi->min_rate); PUT(bi->max_rate); PUT(bi->reservoir_bits);
  PUT(bi->reservoir_bias); PUT(bi->slew_damp); PUT(vi.bitrate_window);
  vorbis_info_clear(&vi);
  return n;
}
#undef PUT
#undef PUTS

/* psy.c's look (`_vp_psy_init` as vorbis_analysis_init runs it) for psy block `block`, as
   doubles: n firstoc shiftoc eighth_octave_lines total_octave_lines rate m3n[0..3] m_val
   tonecomp_endp tonecomp_thres min_nn_lp endeg_start endeg_end tonefix_end n25p n33p n75p nn75pt
   nn50pt nn25pt, then ath octave bark ntfix_noiseoffset (n each), noiseoffset (3n), tonecurves
   (17 x 8 x 58). Returns the count, the init error, -1 (cap), or -2 (no such block). */
long aotuv_psy_look(int ch, long rate, int managed, float q, long min, long bitrate, long max,
                    int block, double *out, long cap) {
  vorbis_info vi;
  vorbis_dsp_state vd;
  long w = 0;
  int ret;
  vorbis_info_init(&vi);
  ret = !managed ? vorbis_encode_init_vbr(&vi, ch, rate, q)
                 : vorbis_encode_init(&vi, ch, rate, max, bitrate, min);
  if (ret) return ret;
  vorbis_analysis_init(&vd, &vi);
  codec_setup_info *ci = vi.codec_setup;
  if (block >= ci->psys) return -2;
  vorbis_look_psy *p = ((private_state *)vd.backend_state)->psy + block;
  long n = p->n;
  if (25 + 7 * n + P_BANDS * P_LEVELS * (EHMER_MAX + 2) > cap) return -1;
  double h[25] = {p->n, p->firstoc, p->shiftoc, p->eighth_octave_lines, p->total_octave_lines,
                  p->rate, p->m3n[0], p->m3n[1], p->m3n[2], p->m_val, p->tonecomp_endp,
                  p->tonecomp_thres, p->min_nn_lp, p->endeg_start, p->endeg_end, p->tonefix_end,
                  p->n25p, p->n33p, p->n75p, p->nn75pt, p->nn50pt, p->nn25pt, 0, 0, 0};
  for (int i = 0; i < 25; i++) out[w++] = h[i];
  for (long i = 0; i < n; i++) out[w++] = p->ath[i];
  for (long i = 0; i < n; i++) out[w++] = p->octave[i];
  for (long i = 0; i < n; i++) out[w++] = p->bark[i];
  for (long i = 0; i < n; i++) out[w++] = p->ntfix_noiseoffset[i];
  for (int j = 0; j < P_NOISECURVES; j++)
    for (long i = 0; i < n; i++) out[w++] = p->noiseoffset[j][i];
  for (int i = 0; i < P_BANDS; i++)
    for (int m = 0; m < P_LEVELS; m++)
      for (int j = 0; j < EHMER_MAX + 2; j++) out[w++] = p->tonecurves[i][m][j];
  vorbis_dsp_clear(&vd);
  vorbis_info_clear(&vi);
  return w;
}

/* psy.c's per-block functions on a configuration's look (psy block `block`), called as
   mapping0_forward calls them; the arrays are the caller's (n = the look's n). */
typedef struct {
  vorbis_info vi;
  vorbis_dsp_state vd;
  vorbis_look_psy *p;
} psyh;
psyh *aotuv_psy_new(int ch, long rate, int managed, float q, long min, long bitrate, long max,
                    int block) {
  psyh *h = calloc(1, sizeof(*h));
  vorbis_info_init(&h->vi);
  if ((!managed ? vorbis_encode_init_vbr(&h->vi, ch, rate, q)
                : vorbis_encode_init(&h->vi, ch, rate, max, bitrate, min)) ||
      block >= ((codec_setup_info *)h->vi.codec_setup)->psys) {
    vorbis_info_clear(&h->vi);
    free(h);
    return 0;
  }
  vorbis_analysis_init(&h->vd, &h->vi);
  h->p = ((private_state *)h->vd.backend_state)->psy + block;
  return h;
}
void aotuv_psy_free(psyh *h) {
  vorbis_dsp_clear(&h->vd);
  vorbis_info_clear(&h->vi);
  free(h);
}
int aotuv_psy_n(psyh *h) { return h->p->n; }
void aotuv_psy_noisemask(psyh *h, float ncl, const float *logmdct, const float *lastmdct,
                         float *epeak, float *npeak, float *logmask, float poste, int block_mode) {
  _vp_noisemask(h->p, ncl, logmdct, lastmdct, epeak, npeak, logmask, poste, block_mode);
}
void aotuv_psy_tonemask(psyh *h, const float *logfft, float *logmask, float gmax, float lmax) {
  _vp_tonemask(h->p, logfft, logmask, gmax, lmax);
}
void aotuv_psy_offset_and_mix(psyh *h, const float *noise, const float *tone, int offset_select,
                              int bit_managed, float *logmask, float *mdct, float *logmdct,
                              float *lastmdct, float *tempmdct, float low_compand, float *npeak,
                              int end_block, int block_mode, int nw, int lw_block_mode,
                              int lw_no, int impadnum) {
  _vp_offset_and_mix(h->p, noise, tone, offset_select, bit_managed, logmask, mdct, logmdct,
                     lastmdct, tempmdct, low_compand, npeak, end_block, block_mode, nw,
                     lw_block_mode, lw_no, impadnum);
}
float aotuv_psy_loudnoise(psyh *h, float ncl, const float *logmdct, int block_mode, int lw_block_mode) {
  return lb_loudnoise_fix(h->p, ncl, logmdct, block_mode, lw_block_mode);
}
float aotuv_postnoise(float *pcm, int nn, int mode, int lw_mode) {
  return _postnoise_detection(pcm, nn, mode, lw_mode);
}

/* block.c + envelope.c alone: pcm (planar, n per channel) written in chunks of chunk[k]
   samples (cycled), then the end of stream; after each write, every block vorbis_analysis_blockout
   gives (no analysis) is logged as doubles: lW W nW blocktype sequence granulepos pcmend eofflag
   ampmax, then pcmend samples per channel, then the envelope (stretch current curmark cursor,
   the marks up to current, each band filter's state). Returns the count (or the init error, -1: cap). */
long aotuv_blocks(int ch, long rate, int managed, float q, long min, long bitrate, long max,
                  const float *const *pcm, long n, const long *chunk, int nchunk,
                  double *out, long cap) {
  vorbis_info vi;
  vorbis_dsp_state vd;
  vorbis_block vb;
  long w = 0, at = 0, k = 0;
  int ret;
  vorbis_info_init(&vi);
  ret = !managed ? vorbis_encode_init_vbr(&vi, ch, rate, q)
                 : vorbis_encode_init(&vi, ch, rate, max, bitrate, min);
  if (ret) return ret;
  vorbis_analysis_init(&vd, &vi);
  vorbis_block_init(&vd, &vb);
  for (;;) {
    long m = n - at < chunk[k % nchunk] ? n - at : chunk[k % nchunk];
    k++;
    if (m) {
      float **b = vorbis_analysis_buffer(&vd, m);
      for (int c = 0; c < ch; c++) memcpy(b[c], pcm[c] + at, m * sizeof(float));
    }
    vorbis_analysis_wrote(&vd, m);
    at += m;
    while (vorbis_analysis_blockout(&vd, &vb) == 1) {
      vorbis_block_internal *vbi = vb.internal;
      double h[9] = {vb.lW, vb.W, vb.nW, vbi->blocktype, vb.sequence, vb.granulepos, vb.pcmend,
                     vb.eofflag, vbi->ampmax};
      envelope_lookup *ve = ((private_state *)vd.backend_state)->ve;
      long marks = ve->current / ve->searchstep + VE_POST;
      long need = 9 + ch * vb.pcmend + 5 + marks + ve->ch * VE_BANDS * (VE_AMP + VE_NEARDC + 4);
      if (w + need > cap) return -1;
      for (int i = 0; i < 9; i++) out[w++] = h[i];
      for (int c = 0; c < ch; c++)
        for (long i = 0; i < vb.pcmend; i++) out[w++] = vb.pcm[c][i];
      /* the envelope after this blockout */
      out[w++] = ve->stretch; out[w++] = ve->current; out[w++] = ve->curmark;
      out[w++] = ve->cursor; out[w++] = marks;
      for (long i = 0; i < marks; i++) out[w++] = ve->mark[i];
      for (int f = 0; f < ve->ch * VE_BANDS; f++) {
        envelope_filter_state *fs = ve->filter + f;
        for (int i = 0; i < VE_AMP; i++) out[w++] = fs->ampbuf[i];
        for (int i = 0; i < VE_NEARDC; i++) out[w++] = fs->nearDC[i];
        out[w++] = fs->ampptr; out[w++] = fs->nearDC_acc; out[w++] = fs->nearDC_partialacc;
        out[w++] = fs->nearptr;
      }
    }
    if (!m) break;
  }
  vorbis_block_clear(&vb);
  vorbis_dsp_clear(&vd);
  vorbis_info_clear(&vi);
  return w;
}

/* framing.c alone: a stream, packets in, pages out (flush != 0: ogg_stream_flush, else
   ogg_stream_pageout); a page's bytes (header + body) into out, its length returned, 0 if none. */
ogg_stream_state *aotuv_ogg_new(int serialno) {
  ogg_stream_state *os = calloc(1, sizeof(*os));
  ogg_stream_init(os, serialno);
  return os;
}
void aotuv_ogg_packetin(ogg_stream_state *os, unsigned char *data, long bytes, int eos, long long granulepos) {
  ogg_packet op = {data, bytes, 0, eos, granulepos, 0};
  ogg_stream_packetin(os, &op);
}
long aotuv_ogg_page(ogg_stream_state *os, int flush, unsigned char *out, long cap) {
  ogg_page og;
  if (!(flush ? ogg_stream_flush(os, &og) : ogg_stream_pageout(os, &og))) return 0;
  if (og.header_len + og.body_len > cap) return -1;
  memcpy(out, og.header, og.header_len);
  memcpy(out + og.header_len, og.body, og.body_len);
  return og.header_len + og.body_len;
}
void aotuv_ogg_free(ogg_stream_state *os) {
  ogg_stream_clear(os);
  free(os);
}

/* mdct.c: the forward transform of n inputs to n/2 outputs; trig/bitrev for the table tests. */
mdct_lookup *aotuv_mdct_new(int n) {
  mdct_lookup *m = calloc(1, sizeof(*m));
  mdct_init(m, n);
  return m;
}
void aotuv_mdct_forward(mdct_lookup *m, float *in, float *out) { mdct_forward(m, in, out); }
const float *aotuv_mdct_trig(mdct_lookup *m) { return m->trig; }
const int *aotuv_mdct_bitrev(mdct_lookup *m) { return m->bitrev; }
void aotuv_mdct_free(mdct_lookup *m) {
  mdct_clear(m);
  free(m);
}

/* smallft.c: the real forward FFT in place; trigcache (3n) and splitcache (32). */
drft_lookup *aotuv_drft_new(int n) {
  drft_lookup *l = calloc(1, sizeof(*l));
  drft_init(l, n);
  return l;
}
void aotuv_drft_forward(drft_lookup *l, float *data) { drft_forward(l, data); }
const float *aotuv_drft_trig(drft_lookup *l) { return l->trigcache; }
const int *aotuv_drft_split(drft_lookup *l) { return l->splitcache; }
void aotuv_drft_free(drft_lookup *l) {
  drft_clear(l);
  free(l);
}
#endif
