//! Bound-port bookkeeping (filled by Device).

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BoundPorts {
    pub arc: u16,
    pub cmc: u16,
    pub flows_control: Option<u16>,
    pub info: u16,
    pub mdns: Option<u16>,
    pub ptp_event: Option<u16>,
    pub ptp_general: Option<u16>,
    pub media: Vec<u16>,
}
