/* Image table shared between the generated tables and the loader. */
#ifndef FASTPROM_IMAGE_H
#define FASTPROM_IMAGE_H

#include <stdint.h>

#define FASTPROM_IMAGE_MAGIC   0x4650524Du  /* 'FPRM' */
#define FASTPROM_IMAGE_VERSION 1u

typedef struct {
    const char    *name;   /* source filename, for diagnostics */
    uint32_t       crc32;  /* over exactly depth bytes of data */
    const uint8_t *data;
} fastprom_image_t;

typedef struct {
    uint32_t                magic;       /* 0x4650524d, 'FPRM' */
    uint32_t                version;
    const char             *part;        /* target part name */
    uint32_t                depth;       /* words per image */
    uint32_t                width;       /* bits per word, 4 or 8 */
    uint32_t                image_count; /* power of two */
    const fastprom_image_t *images;
} fastprom_image_table_t;

#endif /* FASTPROM_IMAGE_H */
