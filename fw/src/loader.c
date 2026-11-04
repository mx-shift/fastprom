/* Image loader: fill the fabric's block RAM from flash, then let the socket
 * outputs go live.
 *
 * This only runs in BRAM mode. In LOGIC mode the contents are part of the
 * fabric bitstream, the outputs are valid the instant configuration
 * completes, and there is nothing for the core to do.
 *
 * The ordering here is the whole point of the file. A bipolar PROM socket is
 * usually on a bus with other devices, and driving a wrong value onto it is
 * worse than driving nothing: at best the host reads garbage, at worst two
 * devices fight. So the outputs stay released until an image has been loaded
 * AND its CRC checked, and any failure leaves them released rather than
 * serving something unverified.
 */
#include "fastprom_fabric.h"
#include "fastprom_image.h"
#include "fastprom_crc32.h"
#include "fastprom_loader.h"

fastprom_status_t fastprom_load(const fastprom_image_table_t *table,
                                uint32_t bank)
{
    fastprom_fabric_t *fab = FASTPROM_FABRIC;

    /* Confirm the fabric is configured and is our logic before writing to
     * it. An unconfigured fabric reads as zero, and the AG32 configures from
     * flash at power-on only - so this is a real failure mode, not a
     * theoretical one. */
    if (fab->id != FASTPROM_FABRIC_MAGIC)
        return FASTPROM_ERR_NO_FABRIC;

    if (table->magic != FASTPROM_IMAGE_MAGIC)
        return FASTPROM_ERR_BAD_TABLE;
    if (table->version != FASTPROM_IMAGE_VERSION)
        return FASTPROM_ERR_BAD_VERSION;
    if (table->image_count == 0u || bank >= table->image_count)
        return FASTPROM_ERR_BAD_BANK;

    const fastprom_image_t *img = &table->images[bank];

    /* Verify before serving, not after. */
    if (fastprom_crc32(img->data, table->depth) != img->crc32)
        return FASTPROM_ERR_BAD_CRC;

    /* DATA auto-increments the pointer, so the whole image is one store per
     * word with no address bookkeeping. */
    fab->ctrl = 0u;
    fab->addr = 0u;
    for (uint32_t i = 0; i < table->depth; i++)
        fab->data = img->data[i];

    /* Read the pointer back: after a full image it must have advanced
     * exactly depth words. This catches a fabric that is present but not
     * accepting writes, which would otherwise show up as a host reading
     * whatever the block RAM powered up holding. */
    if ((fab->addr & (table->depth - 1u)) != 0u)
        return FASTPROM_ERR_LOAD_FAILED;

    fab->ctrl = FASTPROM_CTRL_READY;
    return FASTPROM_OK;
}
