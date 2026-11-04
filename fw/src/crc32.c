/* CRC-32/ISO-HDLC, matching Python's zlib.crc32 so that the value computed
 * here can be compared against the one tools/mkimagetable.py baked into the
 * image table.
 *
 * Bitwise rather than table-driven: this runs once per image at boot, where a
 * few hundred microseconds is free, and a 1 KiB table is not worth spending
 * on a part with 256 KiB of flash shared with the fabric bitstream.
 */
#include "fastprom_crc32.h"

uint32_t fastprom_crc32(const uint8_t *data, uint32_t len)
{
    uint32_t crc = 0xFFFFFFFFu;

    for (uint32_t i = 0; i < len; i++) {
        crc ^= data[i];
        for (int bit = 0; bit < 8; bit++) {
            uint32_t mask = -(crc & 1u);
            crc = (crc >> 1) ^ (0xEDB88320u & mask);
        }
    }

    return ~crc;
}
