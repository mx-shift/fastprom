//! The update page's WebAssembly: `fastprom-host`'s procedures over
//! WebUSB and Web Serial, and the simulated board, for `web/js/`
//! (`docs/12-web.md`).
//!
//! WebUSB itself stays in JavaScript (`web/js/usb.js`): it finds the
//! update interface and hands this module a `Link` with three methods, one
//! per [`Transport`] method. Web Serial likewise (`web/js/serial.js`): a
//! `SerialLink` with four, one per [`SerialPort`] method. Structured
//! results cross as JSON strings (`serde_json`), bytes as `Uint8Array`, and
//! errors as JavaScript `Error`s whose `name` is `Gone` when the device went
//! away (unplugged, rebooting, power lost), `Refused` when it refused a
//! command, `NoAnswer` and `WrongDevice` for a recovery that found no boot
//! loader or another chip, and `Error` otherwise.

use js_sys::{Function, Promise, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use fastprom_core::crc32::crc32;
use fastprom_core::header::{BoardRev, Part, Version};
use fastprom_host::an3155::Loader;
use fastprom_host::bundle::{self, Bundle};
use fastprom_host::client::{Client, Error};
use fastprom_host::info::DeviceInfo;
use fastprom_host::recovery::{self, FullFiles, RecoveryError, RecoveryImage};
use fastprom_host::rom::{self, Geometry, Rom};
use fastprom_host::serial::{SerialError, SerialPort};
use fastprom_host::sim::{SimConfig, SimDevice, UsbError, WireFault};
use fastprom_host::table::{self, Plan, Slot};
use fastprom_host::transport::{Transport, TransportError};
use fastprom_host::update;

// The reset request's bRequest, passed to usb.js, which builds the setup.
use fastprom_core::update::proto::REQUEST_RESET;

// ---- the link -------------------------------------------------------------------

#[wasm_bindgen]
extern "C" {
    /// `web/js/usb.js`'s `UpdateLink`, over a claimed interface. Each
    /// method's promise rejects (with a `NetworkError` or `NotFoundError`
    /// for a device that went away) when the transfer did not happen.
    pub type Link;

    /// Resolves `"ok"`, or another WebUSB status (`"stall"`). Copies
    /// `data` before it awaits.
    #[wasm_bindgen(method, js_name = bulkOut)]
    fn bulk_out(this: &Link, data: &[u8]) -> Promise;

    /// Resolves a `Uint8Array`, or `null` for a halted endpoint.
    #[wasm_bindgen(method, js_name = bulkIn)]
    fn bulk_in(this: &Link, len: u32) -> Promise;

    /// The protocol's reset: a vendor request to the interface, host to
    /// device, no data. As `bulkOut`.
    #[wasm_bindgen(method)]
    fn reset(this: &Link, request: u8) -> Promise;
}

/// A rejected transfer: gone, or something else.
fn rejected(e: &JsValue) -> TransportError {
    let name = js_sys::Reflect::get(e, &"name".into())
        .ok()
        .and_then(|n| n.as_string())
        .unwrap_or_default();
    if matches!(name.as_str(), "NetworkError" | "NotFoundError" | "Gone") {
        return TransportError::Gone;
    }
    let msg = js_sys::Reflect::get(e, &"message".into())
        .ok()
        .and_then(|m| m.as_string())
        .or_else(|| e.as_string())
        .unwrap_or_else(|| "a transfer failed".into());
    TransportError::Other(msg)
}

async fn status(p: Promise) -> Result<(), TransportError> {
    let v = JsFuture::from(p).await.map_err(|e| rejected(&e))?;
    match v.as_string().as_deref() {
        Some("ok") => Ok(()),
        Some("stall") => Err(TransportError::Other("the endpoint is halted".into())),
        other => Err(TransportError::Other(format!("transfer status {other:?}"))),
    }
}

async fn data(p: Promise) -> Result<Vec<u8>, TransportError> {
    let v = JsFuture::from(p).await.map_err(|e| rejected(&e))?;
    if v.is_null() {
        return Err(TransportError::Other("the endpoint is halted".into()));
    }
    v.dyn_into::<Uint8Array>()
        .map(|a| a.to_vec())
        .map_err(|_| TransportError::Other("a transfer returned no bytes".into()))
}

/// [`Transport`] over a [`Link`].
struct Web(Link);

impl Transport for Web {
    async fn bulk_out(&mut self, d: &[u8]) -> Result<(), TransportError> {
        status(self.0.bulk_out(d)).await
    }
    async fn bulk_in(&mut self, len: usize) -> Result<Vec<u8>, TransportError> {
        data(self.0.bulk_in(len as u32)).await
    }
    async fn reset(&mut self) -> Result<(), TransportError> {
        status(self.0.reset(REQUEST_RESET)).await
    }
}

fn client(link: Link) -> Client<Web> {
    Client::new(Web(link))
}

// ---- the serial link ----------------------------------------------------------

#[wasm_bindgen]
extern "C" {
    /// `web/js/serial.js`'s `SerialLink`, over an open Web Serial port at
    /// 115200 8E1 (or the simulator's). Each method's promise rejects (a
    /// `NetworkError`, or one named `Gone`) when the port went away.
    pub type SerialLink;

    /// Resolves when `data` (copied before it awaits) is written.
    #[wasm_bindgen(method)]
    fn write(this: &SerialLink, data: &[u8]) -> Promise;

    /// Resolves a `Uint8Array` of at most `len` bytes: as soon as that many
    /// arrived, or what arrived in `timeout_ms`.
    #[wasm_bindgen(method)]
    fn read(this: &SerialLink, len: u32, timeout_ms: u32) -> Promise;

    /// Resolves once what was received and not read is dropped.
    #[wasm_bindgen(method)]
    fn discard(this: &SerialLink) -> Promise;

    /// Resolves after `ms` milliseconds.
    #[wasm_bindgen(method)]
    fn sleep(this: &SerialLink, ms: u32) -> Promise;
}

fn serial_rejected(e: &JsValue) -> SerialError {
    match rejected(e) {
        TransportError::Gone => SerialError::Gone,
        TransportError::Other(m) => SerialError::Other(m),
    }
}

/// [`SerialPort`] over a [`SerialLink`].
struct WebSerial(SerialLink);

impl SerialPort for WebSerial {
    async fn write(&mut self, data: &[u8]) -> Result<(), SerialError> {
        JsFuture::from(self.0.write(data))
            .await
            .map(|_| ())
            .map_err(|e| serial_rejected(&e))
    }
    async fn read(&mut self, len: usize, timeout_ms: u32) -> Result<Vec<u8>, SerialError> {
        let len = u32::try_from(len).unwrap_or(u32::MAX);
        let v = JsFuture::from(self.0.read(len, timeout_ms))
            .await
            .map_err(|e| serial_rejected(&e))?;
        v.dyn_into::<Uint8Array>()
            .map(|a| a.to_vec())
            .map_err(|_| SerialError::Other("a serial read returned no bytes".into()))
    }
    async fn discard(&mut self) -> Result<(), SerialError> {
        JsFuture::from(self.0.discard())
            .await
            .map(|_| ())
            .map_err(|e| serial_rejected(&e))
    }
    async fn sleep(&mut self, ms: u32) {
        // A sleep that fails is a sleep that did not happen: the protocol's
        // timeouts still hold.
        let _ = JsFuture::from(self.0.sleep(ms)).await;
    }
}

// ---- errors -----------------------------------------------------------------------

fn js_error(name: &str, msg: &str) -> JsValue {
    let e = js_sys::Error::new(msg);
    e.set_name(name);
    e.into()
}

fn fail(e: &Error) -> JsValue {
    let name = if e.is_gone() {
        "Gone"
    } else if e.status().is_some() {
        "Refused"
    } else {
        "Error"
    };
    js_error(name, &e.to_string())
}

fn bad(msg: impl AsRef<str>) -> JsValue {
    js_error("Error", msg.as_ref())
}

fn recovery_failed(e: &RecoveryError) -> JsValue {
    let name = match e {
        RecoveryError::NoAnswer => "NoAnswer",
        RecoveryError::WrongDevice(_) => "WrongDevice",
        RecoveryError::Gone => "Gone",
        _ => "Error",
    };
    js_error(name, &e.to_string())
}

fn known_parts(json: &str) -> Result<Vec<String>, JsValue> {
    serde_json::from_str(json).map_err(|e| bad(format!("the part list: {e}")))
}

fn json<T: serde::Serialize>(v: &T) -> Result<String, JsValue> {
    serde_json::to_string(v).map_err(|e| bad(e.to_string()))
}

fn progress_fn(f: &Function) -> impl FnMut(&str, u32, u32) + '_ {
    move |step, done, total| {
        // A progress display that throws is no reason to stop an update.
        let _ = f.call3(
            &JsValue::NULL,
            &JsValue::from_str(step),
            &JsValue::from(done),
            &JsValue::from(total),
        );
    }
}

fn device_info(info_json: &str) -> Result<DeviceInfo, JsValue> {
    serde_json::from_str(info_json).map_err(|e| bad(format!("device info: {e}")))
}

fn parse_bundle(json: &str) -> Result<Bundle, JsValue> {
    bundle::parse(json).map_err(bad)
}

/// What the device enumerates as (fastprom-usb's numbers), as JSON:
/// `vid`, `pid`, `bcdDevice` for each firmware, the update interface's
/// number and its reset request's `bRequest`.
#[wasm_bindgen(js_name = usbIds)]
pub fn usb_ids() -> String {
    use fastprom_host::sim::{BCD_APPLICATION, BCD_DFU, PID, VID};
    use fastprom_usb::ids::UPDATE_INTERFACE;
    format!(
        r#"{{"vid":{VID},"pid":{PID},"bcdApplication":{BCD_APPLICATION},"bcdDfu":{BCD_DFU},"updateInterface":{UPDATE_INTERFACE},"requestReset":{REQUEST_RESET}}}"#
    )
}

/// A firmware's configuration descriptor as it sends it: the DFU stage's
/// (`dfu`) or the application's. For the page's tests of how it finds the
/// update interface.
#[wasm_bindgen(js_name = firmwareConfiguration)]
pub fn firmware_configuration(dfu: bool) -> Vec<u8> {
    if dfu {
        fastprom_usb::desc::CONFIG_DFU.to_vec()
    } else {
        fastprom_usb::desc::CONFIG_APP.to_vec()
    }
}

// ---- the procedures ---------------------------------------------------------------

/// What [`connect`] found.
#[wasm_bindgen]
pub struct Connected {
    info: String,
    table: Option<Vec<u8>>,
}

#[wasm_bindgen]
impl Connected {
    /// `INFO`, as JSON (`fastprom_host::info::DeviceInfo`).
    #[wasm_bindgen(getter)]
    pub fn info(&self) -> String {
        self.info.clone()
    }

    /// The image table read back, if `INFO` said it is valid.
    #[wasm_bindgen(getter)]
    pub fn table(&self) -> Option<Vec<u8>> {
        self.table.clone()
    }
}

/// Reset the interface, `INFO`, and read the image table.
///
/// # Errors
///
/// A `Gone`, `Refused` or `Error`.
#[wasm_bindgen]
pub async fn connect(link: Link) -> Result<Connected, JsValue> {
    let c = update::connect(&mut client(link))
        .await
        .map_err(|e| fail(&e))?;
    Ok(Connected {
        info: json(&c.info)?,
        table: c.table,
    })
}

/// Write an image table and read it back (not `LOAD`).
///
/// # Errors
///
/// A `Gone`, `Refused` or `Error`.
#[wasm_bindgen(js_name = writeTable)]
pub async fn write_table(link: Link, table: Vec<u8>, progress: Function) -> Result<(), JsValue> {
    let mut p = progress_fn(&progress);
    update::write_table(&mut client(link), &table, &mut p)
        .await
        .map_err(|e| fail(&e))
}

/// `LOAD`.
///
/// # Errors
///
/// A `Gone`, `Refused` or `Error`.
#[wasm_bindgen]
pub async fn load(link: Link) -> Result<(), JsValue> {
    update::load(&mut client(link)).await.map_err(|e| fail(&e))
}

/// `REBOOT` into DFU mode.
///
/// # Errors
///
/// A `Refused` or `Error`.
#[wasm_bindgen(js_name = enterDfu)]
pub async fn enter_dfu(link: Link) -> Result<(), JsValue> {
    update::enter_dfu(&mut client(link))
        .await
        .map_err(|e| fail(&e))
}

/// `REBOOT` into the application.
///
/// # Errors
///
/// A `Refused` or `Error`.
#[wasm_bindgen(js_name = rebootApplication)]
pub async fn reboot_application(link: Link) -> Result<(), JsValue> {
    update::reboot_application(&mut client(link))
        .await
        .map_err(|e| fail(&e))
}

/// Check the bundle's firmware file against its manifest; its header's
/// version.
///
/// # Errors
///
/// An `Error` saying what does not hold.
#[wasm_bindgen(js_name = checkFirmware)]
pub fn check_firmware(bundle_json: &str, file: &[u8]) -> Result<String, JsValue> {
    let b = parse_bundle(bundle_json)?;
    let entry = b
        .firmware
        .as_ref()
        .ok_or_else(|| bad("no firmware in the bundle"))?;
    let app = bundle::check_firmware(entry, file).map_err(bad)?;
    Ok(app.header.fw_version.to_string())
}

/// Write the application in DFU mode and `COMMIT` it. `info_json` is the
/// DFU stage's `INFO`.
///
/// # Errors
///
/// A `Gone`, `Refused` or `Error`.
#[wasm_bindgen(js_name = updateFirmware)]
pub async fn update_firmware(
    link: Link,
    info_json: String,
    bundle_json: String,
    file: Vec<u8>,
    progress: Function,
) -> Result<(), JsValue> {
    let info = device_info(&info_json)?;
    let b = parse_bundle(&bundle_json)?;
    let entry = b
        .firmware
        .as_ref()
        .ok_or_else(|| bad("no firmware in the bundle"))?;
    let app = bundle::check_firmware(entry, &file).map_err(bad)?;
    let mut p = progress_fn(&progress);
    update::update_firmware(&mut client(link), &info, &app, &mut p)
        .await
        .map_err(|e| fail(&e))
}

/// Write `part`'s fabric in DFU mode and `COMMIT` it.
///
/// # Errors
///
/// A `Gone`, `Refused` or `Error`.
#[wasm_bindgen(js_name = updateFabric)]
pub async fn update_fabric(
    link: Link,
    info_json: String,
    bundle_json: String,
    part: String,
    bitstream: Vec<u8>,
    page: Vec<u8>,
    progress: Function,
) -> Result<(), JsValue> {
    let info = device_info(&info_json)?;
    let b = parse_bundle(&bundle_json)?;
    let entry = b
        .fabrics
        .iter()
        .find(|f| f.part == part)
        .ok_or_else(|| bad(format!("no fabric for {part} in the bundle")))?;
    let fabric = bundle::check_fabric(entry, &bitstream, &page).map_err(bad)?;
    let mut p = progress_fn(&progress);
    update::update_fabric(&mut client(link), &info, &fabric, &mut p)
        .await
        .map_err(|e| fail(&e))
}

// ---- bundles, tables and files ---------------------------------------------------

/// The bundle's manifest, checked, as JSON.
///
/// # Errors
///
/// An `Error` for a manifest this page does not read.
#[wasm_bindgen(js_name = parseBundle)]
pub fn parse_bundle_js(json: &str) -> Result<String, JsValue> {
    let b = parse_bundle(json)?;
    self::json(&b)
}

/// What the bundle offers this device (`fastprom_host::bundle::UpdatePlan`
/// as JSON). `part` overrides the part the device reports; `known_json` is
/// the page's part keys (`parts.json`'s), a JSON array.
///
/// # Errors
///
/// An `Error` for input that does not parse.
#[wasm_bindgen(js_name = planUpdate)]
pub fn plan_update(
    bundle_json: &str,
    info_json: &str,
    part: Option<String>,
    known_json: &str,
) -> Result<String, JsValue> {
    let b = parse_bundle(bundle_json)?;
    let info = device_info(info_json)?;
    let known = known_parts(known_json)?;
    json(&bundle::plan(&b, &info, part.as_deref(), &known))
}

/// The part the board reports, as JSON `{"part", "source"}` (its fabric
/// manifest's, or its image table's when the manifest is missing), or
/// `null`. `known_json` as for [`plan_update`].
///
/// # Errors
///
/// An `Error` for input that does not parse.
#[wasm_bindgen(js_name = boardPart)]
pub fn board_part(info_json: &str, known_json: &str) -> Result<String, JsValue> {
    let info = device_info(info_json)?;
    let known = known_parts(known_json)?;
    Ok(match bundle::board_part(&info, &known) {
        Some((part, source)) => serde_json::json!({ "part": part, "source": source }).to_string(),
        None => "null".into(),
    })
}

// ---- recovery -----------------------------------------------------------------------

/// What recovery would write from the bundle for `part` (or none), as
/// JSON (`fastprom_host::recovery::RecoveryPlan`).
///
/// # Errors
///
/// An `Error` for a manifest this page does not read.
#[wasm_bindgen(js_name = recoveryPlan)]
pub fn recovery_plan(bundle_json: &str, part: Option<String>) -> Result<String, JsValue> {
    let b = parse_bundle(bundle_json)?;
    json(&recovery::plan(&b, part.as_deref()))
}

/// Recover the board on `link`'s serial port from the bundle's files:
/// `loader`, `dfu` and `options` always; `part`'s fabric (`fabric`,
/// `page`) and the firmware (`app`) when `part` is given. Resolves the
/// report as JSON (`fastprom_host::recovery::Report`).
///
/// # Errors
///
/// An `Error` for files that do not check (nothing is sent then); a
/// `NoAnswer`, `WrongDevice`, `Gone` or `Error` from the board.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub async fn recover(
    link: SerialLink,
    bundle_json: String,
    part: Option<String>,
    loader: Vec<u8>,
    dfu: Vec<u8>,
    options: Vec<u8>,
    app: Vec<u8>,
    fabric: Vec<u8>,
    page: Vec<u8>,
    progress: Function,
) -> Result<String, JsValue> {
    let b = parse_bundle(&bundle_json)?;
    let full = part.as_deref().map(|part| FullFiles {
        part,
        app: &app,
        fabric: &fabric,
        page: &page,
    });
    let img = RecoveryImage::check(&b, &loader, &dfu, &options, full).map_err(bad)?;
    let mut p = progress_fn(&progress);
    let report = recovery::recover(&mut Loader::new(WebSerial(link)), &img, &mut p)
        .await
        .map_err(|e| recovery_failed(&e))?;
    json(&report)
}

#[derive(serde::Serialize)]
struct ImageSummary {
    name: String,
    crc: u32,
    /// Words the file gave; the rest are blank padding.
    given: usize,
    padded: usize,
    /// Its words match its CRC (a table cut short by a power loss may not).
    intact: bool,
}

impl From<&Rom> for ImageSummary {
    fn from(r: &Rom) -> Self {
        Self {
            name: r.name.clone(),
            crc: r.crc,
            given: r.given,
            padded: r.padded(),
            intact: crc32(&r.words) == r.crc,
        }
    }
}

#[derive(serde::Serialize)]
struct TableSummary {
    part: String,
    depth: u32,
    width: u8,
    images: Vec<ImageSummary>,
}

/// The images of a table read from a device, as JSON.
///
/// # Errors
///
/// An `Error` for a table the firmware would not serve.
#[wasm_bindgen(js_name = tableImages)]
pub fn table_images(table: &[u8]) -> Result<String, JsValue> {
    let (g, part, images) = table::current(table).map_err(bad)?;
    json(&TableSummary {
        part,
        depth: g.depth,
        width: g.width,
        images: images
            .iter()
            .map(|i| ImageSummary {
                name: i.name.clone(),
                crc: i.crc,
                given: i.words.len(),
                padded: 0,
                intact: crc32(&i.words) == i.crc,
            })
            .collect(),
    })
}

/// The page's image-table plan: a ROM file, a kept image or nothing per
/// switch position.
#[wasm_bindgen]
pub struct TableBuilder(Plan);

#[wasm_bindgen]
impl TableBuilder {
    /// For a part of `depth` words of `width` bits.
    ///
    /// # Errors
    ///
    /// An `Error` for an organisation the table cannot hold.
    #[wasm_bindgen(constructor)]
    pub fn new(part: &str, depth: u32, width: u8) -> Result<Self, JsValue> {
        let g = Geometry::new(depth, width).map_err(bad)?;
        Ok(Self(Plan::new(part, g)))
    }

    fn slot(&mut self, position: usize) -> Result<&mut Slot, JsValue> {
        self.0
            .slots
            .get_mut(position)
            .ok_or_else(|| bad(format!("switch position {position}")))
    }

    /// Position `position` gets a ROM file; its summary as JSON.
    ///
    /// # Errors
    ///
    /// An `Error` saying what is wrong with the file.
    #[wasm_bindgen(js_name = setFile)]
    pub fn set_file(
        &mut self,
        position: usize,
        name: &str,
        bytes: &[u8],
    ) -> Result<String, JsValue> {
        let r =
            rom::parse(name, bytes, self.0.geometry).map_err(|e| bad(format!("{name}: {e}")))?;
        let s = json(&ImageSummary::from(&r))?;
        *self.slot(position)? = Slot::Rom(r);
        Ok(s)
    }

    /// Position `position` keeps image `index` of the table read from the
    /// device.
    ///
    /// # Errors
    ///
    /// An `Error` for a table of another organisation, or no such image.
    #[wasm_bindgen(js_name = setKeep)]
    pub fn set_keep(
        &mut self,
        position: usize,
        table: &[u8],
        index: usize,
    ) -> Result<String, JsValue> {
        let r = table::kept(table, index, self.0.geometry).map_err(bad)?;
        let s = json(&ImageSummary::from(&r))?;
        *self.slot(position)? = Slot::Rom(r);
        Ok(s)
    }

    /// Position `position` holds nothing.
    ///
    /// # Errors
    ///
    /// For a position that is not 0 to 3.
    #[wasm_bindgen(js_name = setEmpty)]
    pub fn set_empty(&mut self, position: usize) -> Result<(), JsValue> {
        *self.slot(position)? = Slot::Empty;
        Ok(())
    }

    /// The table that would be written, as JSON: count, and each image.
    ///
    /// # Errors
    ///
    /// Only if serialising fails.
    pub fn summary(&self) -> Result<String, JsValue> {
        json(&TableSummary {
            part: self.0.part.clone(),
            depth: self.0.geometry.depth,
            width: self.0.geometry.width,
            images: self.0.images().iter().map(ImageSummary::from).collect(),
        })
    }

    /// The table's bytes.
    ///
    /// # Errors
    ///
    /// An `Error` for no image at all, or a table the firmware would refuse.
    pub fn build(&self) -> Result<Vec<u8>, JsValue> {
        self.0.build().map_err(bad)
    }
}

// ---- the simulated board ----------------------------------------------------------

/// The simulated board (`fastprom_host::sim`), for `web/js/sim-usb.js`.
#[wasm_bindgen]
pub struct Sim(SimDevice);

fn usb(e: UsbError) -> JsValue {
    match e {
        UsbError::Gone => js_error("NetworkError", "the device has been disconnected"),
        other => js_error("UsbError", other.as_str()),
    }
}

#[wasm_bindgen]
impl Sim {
    /// A board as provisioned over the UART, for `board_rev` and `part`,
    /// with an application at `major.minor.patch`; powered on.
    ///
    /// # Errors
    ///
    /// An `Error` for a revision, part or organisation that is not one.
    #[wasm_bindgen(constructor)]
    pub fn new(
        board_rev: &str,
        part: &str,
        depth: u32,
        width: u8,
        major: u16,
        minor: u16,
        patch: u16,
    ) -> Result<Self, JsValue> {
        let config = SimConfig {
            board_rev: BoardRev::new(board_rev).ok_or_else(|| bad("board revision"))?,
            dfu_version: Version {
                major: 0,
                minor: 1,
                patch: 0,
            },
            part: Part::new(part).ok_or_else(|| bad("part"))?,
            geometry: Geometry::new(depth, width).map_err(bad)?,
        };
        let app = Version {
            major,
            minor,
            patch,
        };
        Ok(Self(SimDevice::factory(config, app)))
    }

    /// A board never provisioned: blank flash and option bytes, unpowered.
    ///
    /// # Errors
    ///
    /// An `Error` for a revision, part or organisation that is not one.
    pub fn blank(board_rev: &str, part: &str, depth: u32, width: u8) -> Result<Sim, JsValue> {
        let config = SimConfig {
            board_rev: BoardRev::new(board_rev).ok_or_else(|| bad("board revision"))?,
            dfu_version: Version {
                major: 0,
                minor: 1,
                patch: 0,
            },
            part: Part::new(part).ok_or_else(|| bad("part"))?,
            geometry: Geometry::new(depth, width).map_err(bad)?,
        };
        Ok(Self(SimDevice::blank(config)))
    }

    /// BOOT0 bridged to 3.3 V (read at the next reset).
    #[wasm_bindgen(js_name = setBoot0)]
    pub fn set_boot0(&mut self, high: bool) {
        self.0.set_boot0(high);
    }

    /// The device ID the chip's boot loader reports.
    #[wasm_bindgen(js_name = setDeviceId)]
    pub fn set_device_id(&self, id: u32) {
        self.0.set_device_id(id);
    }

    /// A fault on the UART wire: `"corrupt"` (a byte from the host) or
    /// `"drop"` (a byte from the board), `after` bytes from now.
    ///
    /// # Errors
    ///
    /// An `Error` for another kind.
    #[wasm_bindgen(js_name = injectFault)]
    pub fn inject_fault(&mut self, kind: &str, after: u32) -> Result<(), JsValue> {
        let fault = match kind {
            "corrupt" => WireFault::Corrupt { after },
            "drop" => WireFault::Drop { after },
            other => return Err(bad(format!("a fault of kind {other:?}"))),
        };
        self.0.inject_fault(fault);
        Ok(())
    }

    /// The host sends `data` on the UART.
    #[wasm_bindgen(js_name = serialWrite)]
    pub fn serial_write(&mut self, data: &[u8]) {
        self.0.serial_write(data);
    }

    /// Up to `max` bytes the board sent on the UART.
    #[wasm_bindgen(js_name = serialRead)]
    pub fn serial_read(&mut self, max: usize) -> Vec<u8> {
        self.0.serial_read(max)
    }

    /// `ms` of quiet on the UART.
    #[wasm_bindgen(js_name = serialIdle)]
    pub fn serial_idle(&mut self, ms: u32) {
        self.0.serial_idle(ms);
    }

    #[wasm_bindgen(js_name = powerOn)]
    pub fn power_on(&mut self) {
        self.0.power_on();
    }

    #[wasm_bindgen(js_name = powerOff)]
    pub fn power_off(&mut self) {
        self.0.power_off();
    }

    /// Lose power at the `ops`-th flash operation from now.
    #[wasm_bindgen(js_name = cutPowerAfter)]
    pub fn cut_power_after(&self, ops: u32) {
        self.0.cut_power_after(u64::from(ops));
    }

    /// The switch, read at the next boot.
    #[wasm_bindgen(js_name = setSwitch)]
    pub fn set_switch(&self, position: u8) {
        self.0.set_switch(position);
    }

    #[wasm_bindgen(getter)]
    pub fn connected(&self) -> bool {
        self.0.connected()
    }

    #[wasm_bindgen(getter, js_name = bcdDevice)]
    pub fn bcd_device(&self) -> u16 {
        self.0.bcd_device()
    }

    #[wasm_bindgen(getter)]
    pub fn serial(&self) -> String {
        self.0.serial()
    }

    /// The device descriptor the firmware sends now (fastprom-usb's).
    #[wasm_bindgen(js_name = deviceDescriptor)]
    pub fn device_descriptor(&self) -> Vec<u8> {
        self.0.device_descriptor().to_vec()
    }

    /// The configuration descriptor the firmware sends now.
    #[wasm_bindgen(js_name = configurationDescriptor)]
    pub fn configuration_descriptor(&self) -> Vec<u8> {
        self.0.configuration_descriptor().to_vec()
    }

    #[wasm_bindgen(getter, js_name = rebootPending)]
    pub fn reboot_pending(&self) -> bool {
        self.0.reboot_pending()
    }

    #[wasm_bindgen(js_name = completeReboot)]
    pub fn complete_reboot(&mut self) {
        self.0.complete_reboot();
    }

    /// `fastprom_host::sim::SimStatus` as JSON.
    ///
    /// # Errors
    ///
    /// Only if serialising fails.
    pub fn status(&self) -> Result<String, JsValue> {
        json(&self.0.status())
    }

    /// # Errors
    ///
    /// A `NetworkError` off the bus, a `UsbError` `timeout` while a
    /// response is unread.
    #[wasm_bindgen(js_name = bulkOut)]
    pub fn bulk_out(&mut self, data: &[u8]) -> Result<(), JsValue> {
        self.0.bulk_out(data).map_err(usb)
    }

    /// # Errors
    ///
    /// A `NetworkError` off the bus, a `UsbError` `timeout` when there is
    /// nothing to send.
    #[wasm_bindgen(js_name = bulkIn)]
    pub fn bulk_in(&mut self, len: usize) -> Result<Vec<u8>, JsValue> {
        self.0.bulk_in(len).map_err(usb)
    }

    /// The update interface's reset request.
    ///
    /// # Errors
    ///
    /// A `NetworkError` off the bus.
    pub fn reset(&mut self) -> Result<(), JsValue> {
        self.0.reset_interface().map_err(usb)
    }
}
