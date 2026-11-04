/* Register map of the fabric's AHB slave (hdl/fastprom_ahb_slave.v).
 *
 * The AG32's hard core presents an AHB-Lite master to the fabric mapped at
 * 0x60000000 (the External AHB window in the MCU reference manual). The
 * fabric side of it is deliberately tiny - the fabric's real job is the ROM.
 */
#ifndef FASTPROM_FABRIC_H
#define FASTPROM_FABRIC_H

#include <stdint.h>

#define FASTPROM_FABRIC_BASE  0x60000000u

/* Magic in the ID register: 'FPRM'. Reading this back proves the fabric is
 * configured and is the logic we think it is, before anything is written. */
#define FASTPROM_FABRIC_MAGIC 0x4650524Du

typedef struct {
    volatile uint32_t ctrl;  /* +0x00 bit 0: READY, release the socket outputs */
    volatile uint32_t addr;  /* +0x04 load pointer */
    volatile uint32_t data;  /* +0x08 write one word at addr, then addr++ */
    volatile uint32_t id;    /* +0x0C read-only magic */
} fastprom_fabric_t;

/* Built with FASTPROM_FABRIC_MOCK, the register block is redirected to a
 * pointer the host tests own. One hook, in one place, rather than every
 * translation unit having to agree on an address. */
#ifdef FASTPROM_FABRIC_MOCK
extern fastprom_fabric_t *fastprom_fabric_mock;
#define FASTPROM_FABRIC (fastprom_fabric_mock)
#else
#define FASTPROM_FABRIC ((fastprom_fabric_t *)FASTPROM_FABRIC_BASE)
#endif

#define FASTPROM_CTRL_READY (1u << 0)

#endif /* FASTPROM_FABRIC_H */
