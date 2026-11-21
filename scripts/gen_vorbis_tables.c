/* Our own tool (not aoTuV code): emits crates/vorbis-enc/src/{tables,setup_tables,psy_tables}.rs from aoTuV's
 * compiled tables (`just vorbis-tables`). It includes window.c and vorbisenc.c themselves (the
 * latter includes every modes/ and books/ header), so every value is the one the C compiler
 * produced: floats printed with 9 significant digits, doubles with 17 (exact: Rust parses them
 * back to the same value). It includes psy.c too (its tables and masking.h's, `psy`). `window`
 * prints the windows; `setup` every file-scope `static const`
 * of the encoder's setup, listed by scripts/vorbis_decls.py (target/vorbis_decls.h), with its
 * `sizeof` length; pointers become references to the named static they point into (found by
 * address), so the Rust data has the C's sharing. Names are the C names in upper case. */
#include "window.c"
#include "vorbisenc.c"
#include "psy.c"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>

#define LEN(a) ((int)(sizeof(a) / sizeof((a)[0])))

/* ---- registry: every declaration's address, size and name ---- */
typedef struct {
  const void *p;
  size_t size, elsz;
  int array;
  char name[64];
} reg_t;
static reg_t R[4096];
static int NR;
static const char *cur = "?"; /* the declaration being printed, for errors */

static void upper(char *d, const char *s) {
  while ((*d++ = toupper((unsigned char)*s++)));
}

static void reg(const void *p, size_t size, size_t elsz, int array, const char *name) {
  if (NR == (int)(sizeof R / sizeof R[0]) || strlen(name) >= sizeof R[0].name) {
    fprintf(stderr, "gen_vorbis_tables: registry full or name too long at %s\n", name);
    exit(1);
  }
  R[NR].p = p;
  R[NR].size = size;
  R[NR].elsz = elsz;
  R[NR].array = array;
  upper(R[NR].name, name);
  NR++;
}

static const reg_t *find(const void *p) {
  for (int i = 0; i < NR; i++)
    if ((const char *)p >= (const char *)R[i].p && (const char *)p < (const char *)R[i].p + R[i].size)
      return &R[i];
  fprintf(stderr, "gen_vorbis_tables: %s: pointer %p into no declaration\n", cur, p);
  exit(1);
}

/* `&NAME`, `&NAME[k]` (one element), or `&NAME[k..]` (slice; `&[]` for NULL) for pointer p */
static char *ref(const void *p, int slice) {
  static char buf[8][128];
  static int k;
  char *b = buf[k++ % 8];
  if (!p && slice) return strcpy(b, "&[]"); /* single-block setups leave the long-block ones NULL */
  const reg_t *r = find(p);
  size_t off = ((const char *)p - (const char *)r->p) / r->elsz;
  if (!r->array) {
    if (slice) snprintf(b, 128, "core::slice::from_ref(&%s)", r->name);
    else snprintf(b, 128, "&%s", r->name);
  } else if (slice) {
    if (off) snprintf(b, 128, "&%s[%zu..]", r->name, off);
    else snprintf(b, 128, "&%s", r->name);
  } else {
    snprintf(b, 128, "&%s[%zu]", r->name, off);
  }
  return b;
}

static char *opt(const void *p, int slice) {
  static char buf[8][160];
  static int k;
  char *b = buf[k++ % 8];
  if (!p) return strcpy(b, "None");
  snprintf(b, 160, "Some(%s)", ref(p, slice));
  return b;
}

static void need(const void *p, const char *what) {
  if (!p) {
    fprintf(stderr, "gen_vorbis_tables: NULL %s\n", what);
    exit(1);
  }
}

/* ---- scalar and array printers ---- */
static void f32s(const float *v, int n) {
  printf("[");
  for (int i = 0; i < n; i++) printf("%s%.8e", i ? ", " : "", v[i]);
  printf("]");
}
static void f64s(const double *v, int n) {
  printf("[");
  for (int i = 0; i < n; i++) printf("%s%.16e", i ? ", " : "", v[i]);
  printf("]");
}
static void i32s(const int *v, int n) {
  printf("[");
  for (int i = 0; i < n; i++) printf("%s%d", i ? ", " : "", v[i]);
  printf("]");
}
#define I32S(a) i32s(a, LEN(a))
#define F32S(a) f32s(a, LEN(a))

static void head(const char *ty, const char *cname, int n) {
  char up[64];
  cur = cname;
  upper(up, cname);
  if (n < 0) printf("#[rustfmt::skip]\npub(crate) static %s: %s = ", up, ty);
  else printf("#[rustfmt::skip]\npub(crate) static %s: [%s; %d] = [", up, ty, n);
}

/* ---- struct printers ---- */
static void p_codebook(const static_codebook *c) {
  printf("StaticCodebook { dim: %ld, entries: %ld, lengthlist: %s, maptype: %d, q_min: %ld, "
         "q_delta: %ld, q_quant: %d, q_sequencep: %d, quantlist: %s }",
         c->dim, c->entries, ref(c->lengthlist, 1), c->maptype, c->q_min, c->q_delta,
         c->q_quant, c->q_sequencep, c->quantlist ? ref(c->quantlist, 1) : "&[]");
}
static void p_bookblock(const static_bookblock *b) {
  printf("StaticBookblock { books: [");
  for (int i = 0; i < 12; i++) {
    printf("%s[", i ? ", " : "");
    for (int k = 0; k < 4; k++) printf("%s%s", k ? ", " : "", opt(b->books[i][k], 0));
    printf("]");
  }
  printf("] }");
}
static void p_residue0(const vorbis_info_residue0 *r) {
  printf("InfoResidue0 { begin: %ld, end: %ld, grouping: %d, partitions: %d, partvals: %d, "
         "groupbook: %d, secondstages: ", r->begin, r->end, r->grouping, r->partitions,
         r->partvals, r->groupbook);
  I32S(r->secondstages);
  printf(", booklist: ");
  I32S(r->booklist);
  printf(", classmetric1: ");
  I32S(r->classmetric1);
  printf(", classmetric2: ");
  I32S(r->classmetric2);
  printf(" }");
}
static void p_restemplate(const vorbis_residue_template *t) {
  need(t->res, "res");
  printf("ResidueTemplate { res_type: %d, limit_type: %d, grouping: %d, res: %s, book_aux: %s, "
         "book_aux_managed: %s, books_base: %s, books_base_managed: %s }",
         t->res_type, t->limit_type, t->grouping, ref(t->res, 0), opt(t->book_aux, 0),
         opt(t->book_aux_managed, 0), opt(t->books_base, 0), opt(t->books_base_managed, 0));
}
static void p_mapping0(const vorbis_info_mapping0 *m) {
  printf("InfoMapping0 { submaps: %d, chmuxlist: ", m->submaps);
  I32S(m->chmuxlist);
  printf(", floorsubmap: ");
  I32S(m->floorsubmap);
  printf(", residuesubmap: ");
  I32S(m->residuesubmap);
  printf(", coupling_steps: %d, coupling_mag: ", m->coupling_steps);
  I32S(m->coupling_mag);
  printf(", coupling_ang: ");
  I32S(m->coupling_ang);
  printf(" }");
}
static void p_maptemplate(const vorbis_mapping_template *t) {
  need(t->map, "map");
  need(t->res, "res");
  printf("MappingTemplate { map: %s, res: %s }", ref(t->map, 1), ref(t->res, 1));
}
static void p_mode(const vorbis_info_mode *m) {
  printf("InfoMode { blockflag: %d, windowtype: %d, transformtype: %d, mapping: %d }",
         m->blockflag, m->windowtype, m->transformtype, m->mapping);
}
static void p_floor1(const vorbis_info_floor1 *f) {
  printf("InfoFloor1 { partitions: %d, partitionclass: ", f->partitions);
  I32S(f->partitionclass);
  printf(", class_dim: ");
  I32S(f->class_dim);
  printf(", class_subs: ");
  I32S(f->class_subs);
  printf(", class_book: ");
  I32S(f->class_book);
  printf(", class_subbook: [");
  for (int i = 0; i < VIF_CLASS; i++) {
    printf("%s", i ? ", " : "");
    I32S(f->class_subbook[i]);
  }
  printf("], mult: %d, postlist: ", f->mult);
  I32S(f->postlist);
  printf(", maxover: %.8e, maxunder: %.8e, maxerr: %.8e, twofitweight: %.8e, twofitatten: %.8e, "
         "n: %d }", f->maxover, f->maxunder, f->maxerr, f->twofitweight, f->twofitatten, f->n);
}
static void p_psy(const vorbis_info_psy *p) {
  printf("InfoPsy { blockflag: %d, ath_adjatt: %.8e, ath_maxatt: %.8e, tone_masteratt: ",
         p->blockflag, p->ath_adjatt, p->ath_maxatt);
  F32S(p->tone_masteratt);
  printf(", tone_centerboost: %.8e, tone_decay: %.8e, tone_abs_limit: %.8e, toneatt: ",
         p->tone_centerboost, p->tone_decay, p->tone_abs_limit);
  F32S(p->toneatt);
  printf(", noisemaskp: %d, noisemaxsupp: %.8e, noisewindowlo: %.8e, noisewindowhi: %.8e, "
         "noisewindowlomin: %d, noisewindowhimin: %d, noisewindowfixed: %d, noiseoff: [",
         p->noisemaskp, p->noisemaxsupp, p->noisewindowlo, p->noisewindowhi,
         p->noisewindowlomin, p->noisewindowhimin, p->noisewindowfixed);
  for (int i = 0; i < P_NOISECURVES; i++) {
    printf("%s", i ? ", " : "");
    F32S(p->noiseoff[i]);
  }
  printf("], noisecompand: ");
  F32S(p->noisecompand);
  printf(", noisecompand_high: ");
  F32S(p->noisecompand_high);
  printf(", flacint: %.8e, max_curve_db: %.8e, normal_p: %d, normal_start: %d, "
         "normal_partition: %d, normal_thresh: %.16e }", p->flacint, p->max_curve_dB,
         p->normal_p, p->normal_start, p->normal_partition, p->normal_thresh);
}
static void p_psyglobal(const vorbis_info_psy_global *g) {
  printf("InfoPsyGlobal { eighth_octave_lines: %d, preecho_thresh: ", g->eighth_octave_lines);
  F32S(g->preecho_thresh);
  printf(", postecho_thresh: ");
  F32S(g->postecho_thresh);
  printf(", stretch_penalty: %.8e, preecho_minenergy: %.8e, ampmax_att_per_sec: %.8e, "
         "coupling_pkhz: ", g->stretch_penalty, g->preecho_minenergy, g->ampmax_att_per_sec);
  I32S(g->coupling_pkHz);
  printf(", coupling_pointlimit: [");
  I32S(g->coupling_pointlimit[0]);
  printf(", ");
  I32S(g->coupling_pointlimit[1]);
  printf("], coupling_prepointamp: ");
  I32S(g->coupling_prepointamp);
  printf(", coupling_postpointamp: ");
  I32S(g->coupling_postpointamp);
  printf(", sliding_lowpass: [");
  I32S(g->sliding_lowpass[0]);
  printf(", ");
  I32S(g->sliding_lowpass[1]);
  printf("] }");
}
static void p_adjblock(const vp_adjblock *a) {
  printf("VpAdjblock { block: ");
  I32S(a->block);
  printf(" }");
}
static void p_compand(const compandblock *c) {
  printf("Compandblock { data: ");
  I32S(c->data);
  printf(" }");
}
static void p_att3(const att3 *a) {
  printf("Att3 { att: ");
  I32S(a->att);
  printf(", boost: %.8e, decay: %.8e }", a->boost, a->decay);
}
static void p_adjstereo(const adj_stereo *a) {
  printf("AdjStereo { pre: ");
  I32S(a->pre);
  printf(", post: ");
  I32S(a->post);
  printf(", khz: ");
  F32S(a->kHz);
  printf(", lowpass_khz: ");
  F32S(a->lowpasskHz);
  printf(" }");
}
static void p_noiseguard(const noiseguard *g) {
  printf("Noiseguard { lo: %d, hi: %d, fixed: %d }", g->lo, g->hi, g->fixed);
}
static void p_noise3(const noise3 *n) {
  printf("Noise3 { data: [");
  for (int i = 0; i < P_NOISECURVES; i++) {
    printf("%s", i ? ", " : "");
    I32S(n->data[i]);
  }
  printf("] }");
}
static void p_setup(const ve_setup_data_template *s) {
  need(s->rate_mapping, "rate_mapping");
  printf("SetupTemplate {\n    mappings: %d,\n    rate_mapping: %s,\n    quality_mapping: %s,\n"
         "    pre_amp: %s,\n    coupling_restriction: %d,\n    samplerate_min_restriction: %ld,\n"
         "    samplerate_max_restriction: %ld,\n    blocksize_short: %s,\n    blocksize_long: %s,\n",
         s->mappings, ref(s->rate_mapping, 1), ref(s->quality_mapping, 1), ref(s->pre_amp, 1),
         s->coupling_restriction, s->samplerate_min_restriction, s->samplerate_max_restriction,
         ref(s->blocksize_short, 1), ref(s->blocksize_long, 1));
  printf("    psy_tone_masteratt: %s,\n    psy_tone_0db: %s,\n    psy_tone_dbsuppress: %s,\n"
         "    psy_tone_adj_impulse: %s,\n    psy_tone_adj_long: %s,\n    psy_tone_adj_other: %s,\n",
         ref(s->psy_tone_masteratt, 1), ref(s->psy_tone_0dB, 1), ref(s->psy_tone_dBsuppress, 1),
         ref(s->psy_tone_adj_impulse, 1), ref(s->psy_tone_adj_long, 1),
         ref(s->psy_tone_adj_other, 1));
  printf("    psy_noiseguards: %s,\n    psy_noise_bias_impulse: %s,\n    psy_noise_bias_padding: %s,\n"
         "    psy_noise_bias_trans: %s,\n    psy_noise_bias_long: %s,\n    psy_noise_dbsuppress: %s,\n",
         ref(s->psy_noiseguards, 1), ref(s->psy_noise_bias_impulse, 1),
         ref(s->psy_noise_bias_padding, 1), ref(s->psy_noise_bias_trans, 1),
         ref(s->psy_noise_bias_long, 1), ref(s->psy_noise_dBsuppress, 1));
  printf("    psy_noise_compand: %s,\n    psy_noise_compand_short_mapping: %s,\n"
         "    psy_noise_compand_long_mapping: %s,\n    psy_noise_normal_start: [%s, %s],\n"
         "    psy_noise_normal_partition: [%s, %s],\n    psy_noise_normal_thresh: %s,\n",
         ref(s->psy_noise_compand, 1), ref(s->psy_noise_compand_short_mapping, 1),
         ref(s->psy_noise_compand_long_mapping, 1), ref(s->psy_noise_normal_start[0], 1),
         ref(s->psy_noise_normal_start[1], 1), ref(s->psy_noise_normal_partition[0], 1),
         ref(s->psy_noise_normal_partition[1], 1), ref(s->psy_noise_normal_thresh, 1));
  printf("    psy_ath_float: %s,\n    psy_ath_abs: %s,\n    psy_lowpass: %s,\n"
         "    global_params: %s,\n    global_mapping: %s,\n    stereo_modes: %s,\n",
         ref(s->psy_ath_float, 1), ref(s->psy_ath_abs, 1), ref(s->psy_lowpass, 1),
         ref(s->global_params, 1), ref(s->global_mapping, 1), opt(s->stereo_modes, 1));
  printf("    floor_books: %s,\n    floor_params: %s,\n    floor_mappings: %d,\n"
         "    floor_mapping_list: %s,\n    maps: %s,\n}",
         ref(s->floor_books, 1), ref(s->floor_params, 1), s->floor_mappings,
         ref(s->floor_mapping_list, 1), ref(s->maps, 1));
}

/* ---- pass 1: register every declaration; pass 2: print it ---- */
#define ARR(T, name) reg(name, sizeof(name), sizeof(name[0]), 1, #name);
#define ONE(name) reg(&name, sizeof(name), sizeof(name), 0, #name);
#define DECL(kind, name) R_##kind(name)
#define R_char_A ARR_
#define R_long_A ARR_
#define R_int_A ARR_
#define R_float_A ARR_
#define R_double_A ARR_
#define R_PINT_A ARR_
#define R_PSC_A ARR_
#define R_PPSC_A ARR_
#define R_PSETUP_A ARR_
#define R_adj_stereo_A ARR_
#define R_att3_A ARR_
#define R_compandblock_A ARR_
#define R_noise3_A ARR_
#define R_noiseguard_A ARR_
#define R_vorbis_info_floor1_A ARR_
#define R_vorbis_info_mapping0_A ARR_
#define R_vorbis_info_mode_A ARR_
#define R_vorbis_info_psy_global_A ARR_
#define R_vorbis_mapping_template_A ARR_
#define R_vorbis_residue_template_A ARR_
#define R_vp_adjblock_A ARR_
#define R_static_bookblock ONE
#define R_static_codebook ONE
#define R_ve_setup_data_template ONE
#define R_vorbis_info_psy ONE
#define R_vorbis_info_residue0 ONE
#define ARR_(name) ARR(0, name)

static void registry(void) {
#include "vorbis_decls.h"
}
#undef DECL

/* arrays of plain values */
#define NUMS(rty, fmt, cast, name)                                                            \
  {                                                                                           \
    head(rty, #name, LEN(name));                                                              \
    for (int i = 0; i < LEN(name); i++)                                                       \
      printf("%s" fmt ",", i % 16 ? " " : "\n    ", (cast)name[i]);                           \
    printf("\n];\n\n");                                                                       \
  }
/* arrays of structs */
#define STRUCTS(rty, fn, name)                                                                \
  {                                                                                           \
    head(rty, #name, LEN(name));                                                              \
    for (int i = 0; i < LEN(name); i++) {                                                     \
      printf("\n    ");                                                                       \
      fn(&name[i]);                                                                           \
      printf(",");                                                                            \
    }                                                                                         \
    printf("\n];\n\n");                                                                       \
  }
#define SINGLE(rty, fn, name)                                                                 \
  {                                                                                           \
    head(rty, #name, -1);                                                                     \
    fn(&name);                                                                                \
    printf(";\n\n");                                                                          \
  }
#define P_char_A(name) NUMS("u8", "%u", unsigned, name)
#define P_long_A(name) NUMS("i64", "%ld", long, name)
#define P_int_A(name) NUMS("i32", "%d", int, name)
#define P_float_A(name) NUMS("f32", "%.8e", double, name)
#define P_double_A(name) NUMS("f64", "%.16e", double, name)
#define P_PINT_A(name)                                                                        \
  {                                                                                           \
    head("&[i32]", #name, LEN(name));                                                         \
    for (int i = 0; i < LEN(name); i++) {                                                     \
      need(name[i], #name);                                                                   \
      printf("%s%s", i ? ", " : "", ref(name[i], 1));                                         \
    }                                                                                         \
    printf("];\n\n");                                                                         \
  }
/* pointer arrays: trailing NULLs dropped (spare slots, `{0}`), interior ones rejected */
#define P_PSC_A(name)                                                                         \
  {                                                                                           \
    int n = LEN(name);                                                                        \
    while (n && !name[n - 1]) n--;                                                            \
    head("&StaticCodebook", #name, n);                                                        \
    for (int i = 0; i < n; i++) {                                                             \
      need(name[i], #name);                                                                   \
      printf("%s%s,", i % 4 ? " " : "\n    ", ref(name[i], 0));                              \
    }                                                                                         \
    printf("\n];\n\n");                                                                       \
  }
#define P_PPSC_A(name)                                                                        \
  {                                                                                           \
    int n = LEN(name);                                                                        \
    while (n && !name[n - 1]) n--;                                                            \
    head("&[&StaticCodebook]", #name, n);                                                     \
    for (int i = 0; i < n; i++) {                                                             \
      need(name[i], #name);                                                                   \
      printf("%s%s,", i % 4 ? " " : "\n    ", ref(name[i], 1));                              \
    }                                                                                         \
    printf("\n];\n\n");                                                                       \
  }
#define P_PSETUP_A(name)                                                                      \
  {                                                                                           \
    int n = LEN(name);                                                                        \
    while (n && !name[n - 1]) n--;                                                            \
    head("&SetupTemplate", #name, n);                                                         \
    for (int i = 0; i < n; i++) {                                                             \
      need(name[i], #name);                                                                   \
      printf("\n    %s,", ref(name[i], 0));                                                   \
    }                                                                                         \
    printf("\n];\n\n");                                                                       \
  }
#define P_adj_stereo_A(name) STRUCTS("AdjStereo", p_adjstereo, name)
#define P_att3_A(name) STRUCTS("Att3", p_att3, name)
#define P_compandblock_A(name) STRUCTS("Compandblock", p_compand, name)
#define P_noise3_A(name) STRUCTS("Noise3", p_noise3, name)
#define P_noiseguard_A(name) STRUCTS("Noiseguard", p_noiseguard, name)
#define P_vorbis_info_floor1_A(name) STRUCTS("InfoFloor1", p_floor1, name)
#define P_vorbis_info_mapping0_A(name) STRUCTS("InfoMapping0", p_mapping0, name)
#define P_vorbis_info_mode_A(name) STRUCTS("InfoMode", p_mode, name)
#define P_vorbis_info_psy_global_A(name) STRUCTS("InfoPsyGlobal", p_psyglobal, name)
#define P_vorbis_mapping_template_A(name) STRUCTS("MappingTemplate", p_maptemplate, name)
#define P_vorbis_residue_template_A(name) STRUCTS("ResidueTemplate", p_restemplate, name)
#define P_vp_adjblock_A(name) STRUCTS("VpAdjblock", p_adjblock, name)
#define P_static_bookblock(name) SINGLE("StaticBookblock", p_bookblock, name)
#define P_static_codebook(name) SINGLE("StaticCodebook", p_codebook, name)
#define P_ve_setup_data_template(name) SINGLE("SetupTemplate", p_setup, name)
#define P_vorbis_info_psy(name) SINGLE("InfoPsy", p_psy, name)
#define P_vorbis_info_residue0(name) SINGLE("InfoResidue0", p_residue0, name)
#define DECL(kind, name) P_##kind(name)

static void setup(void) {
  registry();
  printf("// @generated by scripts/gen_vorbis_tables.c from aoTuV b6.03 (libvorbis 1.3.7, BSD-3):\n"
         "// vorbisenc.c and its modes/, books/ headers. Do not edit: run `just vorbis-tables`.\n"
         "// Names are the C names in upper case.\n"
         "#![allow(dead_code, clippy::excessive_precision, clippy::approx_constant,\n"
         "    clippy::unreadable_literal)]\n\n"
         "use crate::templates::*;\n\n");
#include "vorbis_decls.h"
}

/* psy.c's tables (and masking.h's ATH and tone masks), by name */
#define FLOATS(name, cname, v, n) { head("f32", cname, n); for (int i_ = 0; i_ < (n); i_++) \
    printf("%s%.8e,", i_ % 8 ? " " : "\n    ", (double)(v)[i_]); printf("\n];\n\n"); }
#define INTS(name, cname, v, n) { head("i32", cname, n); for (int i_ = 0; i_ < (n); i_++) \
    printf("%s%d,", i_ % 16 ? " " : "\n    ", (v)[i_]); printf("\n];\n\n"); }
static void psy(void) {
  printf("// @generated by scripts/gen_vorbis_tables.c from aoTuV b6.03 (libvorbis 1.3.7, BSD-3):\n"
         "// psy.c and masking.h. Do not edit: run `just vorbis-tables`. Names are the C names in\n"
         "// upper case; `tonemasks[P_BANDS][6][EHMER_MAX]` flattened.\n"
         "#![allow(dead_code, clippy::excessive_precision, clippy::approx_constant)]\n\n");
  FLOATS(, "ATH", ATH, LEN(ATH));
  FLOATS(, "tonemasks", &tonemasks[0][0][0], P_BANDS * 6 * EHMER_MAX);
  FLOATS(, "ntfix_offset", ntfix_offset, LEN(ntfix_offset));
  FLOATS(, "FLOOR1_fromdB_LOOKUP", FLOOR1_fromdB_LOOKUP, LEN(FLOOR1_fromdB_LOOKUP));
  INTS(, "m3n32", m3n32, M3C);
  INTS(, "m3n44", m3n44, M3C);
  INTS(, "m3n48", m3n48, M3C);
  INTS(, "m3n32x2", m3n32x2, M3C);
  INTS(, "m3n44x2", m3n44x2, M3C);
  INTS(, "m3n48x2", m3n48x2, M3C);
  INTS(, "freq_bfn128", freq_bfn128, LEN(freq_bfn128));
  INTS(, "freq_bfn256", freq_bfn256, LEN(freq_bfn256));
  INTS(, "stn_compand", stn_compand, LEN(stn_compand));
  {
    double st[LEN(stereo_threshholds)], stx[LEN(stereo_threshholds_X)];
    for (int i = 0; i < LEN(st); i++) st[i] = stereo_threshholds[i];
    for (int i = 0; i < LEN(stx); i++) stx[i] = stereo_threshholds_X[i];
    head("f64", "stereo_threshholds", LEN(st));
    for (int i = 0; i < LEN(st); i++) printf(" %.16e,", st[i]);
    printf("\n];\n\n");
    head("f64", "stereo_threshholds_X", LEN(stx));
    for (int i = 0; i < LEN(stx); i++) printf(" %.16e,", stx[i]);
    printf("\n];\n\n");
  }
  printf("/// `set_aotuv_psy`: (tonecomp_endp, tonecomp_thres, min_nn_lp, tonefix_end).\n");
  head("(i32, f32, i32, i32)", "set_aotuv_psy", LEN(set_aotuv_psy));
  for (int i = 0; i < LEN(set_aotuv_psy); i++)
    printf("\n    (%d, %.8e, %d, %d),", set_aotuv_psy[i].tonecomp_endp, set_aotuv_psy[i].tonecomp_thres,
           set_aotuv_psy[i].min_nn_lp, set_aotuv_psy[i].tonefix_end);
  printf("\n];\n");
}

static void window(void) {
  printf("// @generated by scripts/gen_vorbis_tables.c from aoTuV b6.03 (libvorbis 1.3.7, BSD-3).\n"
         "// Do not edit: run `just vorbis-tables`. Names are the C names in upper case.\n"
         "#![allow(clippy::excessive_precision, clippy::approx_constant)]\n\n");
  for (int i = 0; i < LEN(vwin); i++) {
    int n = 64 << i;
    printf("/// The rising half of the %d-sample Vorbis power-sine window.\n", n);
    printf("#[rustfmt::skip]\npub(crate) static VWIN%d: [f32; %d] = [", n, n / 2);
    for (int k = 0; k < n / 2; k++) printf("%s%.8e,", k % 6 ? " " : "\n    ", vwin[i][k]);
    printf("\n];\n\n");
  }
  printf("/// `vwin`: the windows by `ilog2(blocksize) - 6`.\npub(crate) static VWIN: [&[f32]; %d] = [",
         LEN(vwin));
  for (int i = 0; i < LEN(vwin); i++) printf("%s&VWIN%d", i ? ", " : "", 64 << i);
  printf("];\n");
}

int main(int argc, char **argv) {
  if (argc > 1 && !strcmp(argv[1], "setup")) setup();
  else if (argc > 1 && !strcmp(argv[1], "psy")) psy();
  else window();
  return 0;
}
