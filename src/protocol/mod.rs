#![allow(dead_code)]

//! Pure serialize/deserialize. No I/O.

pub mod arc;
pub mod buf;
pub mod cmc;
pub mod flows_control;
pub mod info_mcast;
pub mod mdns;
pub mod media;
pub mod pcm;
pub mod ptp_v1;
pub mod req_resp;

pub mod ports {
    pub const ARC: u16 = 4440;
    pub const CMC: u16 = 8800;
    pub const FLOWS_CONTROL: u16 = 4455;
    pub const INFO_BIND: u16 = 8700;
    pub const INFO_DEST_PORT: u16 = 8702;
    pub const HEARTBEAT_DEST_PORT: u16 = 8708;
    pub const MDNS: u16 = 5353;
    pub const PTP_EVENT: u16 = 319;
    pub const PTP_GENERAL: u16 = 320;
    pub const MEDIA_PORT_START: u16 = 0x3800;
    pub const MEDIA_PORT_END: u16 = 0x397F;
    pub const MEDIA_PORT_START_2: u16 = 0x3980;
    pub const MEDIA_PORT_END_2: u16 = 0x39FF;

    pub const MDNS_GROUP: [u8; 4] = [224, 0, 0, 251];
    pub const INFO_GROUP: [u8; 4] = [224, 0, 0, 231];
    pub const HEARTBEAT_GROUP: [u8; 4] = [224, 0, 0, 233];
    pub const PTP_GROUP: [u8; 4] = [224, 0, 1, 129];
}

pub const HEADER_RR: usize = 10;
pub const HEADER_MCAST: usize = 32;
pub const OPCODE2_OK: u16 = 1;
pub const OPCODE2_MORE: u16 = 0x8112;
pub const OPCODE2_UNSUPPORTED: u16 = 0x0030;
pub const OPCODE2_FAIL: u16 = 0xFFFF;
