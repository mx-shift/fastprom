#ifndef FASTPROM_LOADER_H
#define FASTPROM_LOADER_H

#include <stdint.h>
#include "fastprom_image.h"

typedef enum {
    FASTPROM_OK = 0,
    FASTPROM_ERR_NO_FABRIC,    /* fabric absent or not our bitstream */
    FASTPROM_ERR_BAD_TABLE,    /* image table magic wrong */
    FASTPROM_ERR_BAD_VERSION,  /* image table format too new */
    FASTPROM_ERR_BAD_BANK,     /* requested image does not exist */
    FASTPROM_ERR_BAD_CRC,      /* image is corrupt */
    FASTPROM_ERR_LOAD_FAILED,  /* fabric did not accept the writes */
} fastprom_status_t;

/* Load image `bank` into the fabric and release the socket outputs.
 * On any error the outputs are left released - never driven with
 * unverified data. */
fastprom_status_t fastprom_load(const fastprom_image_table_t *table,
                                uint32_t bank);

#endif /* FASTPROM_LOADER_H */
