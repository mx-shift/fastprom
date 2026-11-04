#ifndef FASTPROM_CRC32_H
#define FASTPROM_CRC32_H

#include <stdint.h>

/* CRC-32/ISO-HDLC (reflected, poly 0xEDB88320, init and xorout 0xFFFFFFFF).
 * Identical to Python zlib.crc32. */
uint32_t fastprom_crc32(const uint8_t *data, uint32_t len);

#endif /* FASTPROM_CRC32_H */
