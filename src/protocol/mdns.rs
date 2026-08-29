//! mDNS / DNS-SD message builder and parser (RFC 1035 / 6762 / 6763).

use std::net::Ipv4Addr;

pub const QTYPE_A: u16 = 1;
pub const QTYPE_PTR: u16 = 12;
pub const QTYPE_TXT: u16 = 16;
pub const QTYPE_SRV: u16 = 33;
pub const QTYPE_ANY: u16 = 255;
pub const CLASS_IN: u16 = 1;
pub const CLASS_FLUSH: u16 = 0x8001; // IN + cache flush
pub const TTL_HOST: u32 = 4500;

pub const FLAGS_QUERY: u16 = 0;
pub const FLAGS_RESPONSE: u16 = 0x8400; // QR + AA

#[derive(Clone, Debug)]
pub struct Question {
    pub name: String,
    pub qtype: u16,
}

#[derive(Clone, Debug)]
pub enum RecordData {
    A(Ipv4Addr),
    Ptr(String),
    Txt(Vec<Vec<u8>>),
    Srv {
        priority: u16,
        weight: u16,
        port: u16,
        target: String,
    },
}

#[derive(Clone, Debug)]
pub struct Record {
    pub name: String,
    pub rtype: u16,
    pub class: u16,
    pub ttl: u32,
    pub data: RecordData,
}

#[derive(Clone, Debug, Default)]
pub struct Message {
    pub id: u16,
    pub flags: u16,
    pub questions: Vec<Question>,
    pub answers: Vec<Record>,
    pub additionals: Vec<Record>,
}

fn write_name(buf: &mut Vec<u8>, name: &str) {
    let n = name.trim_end_matches('.');
    if n.is_empty() {
        buf.push(0);
        return;
    }
    for label in n.split('.') {
        let bytes = label.as_bytes();
        buf.push(bytes.len() as u8);
        buf.extend_from_slice(bytes);
    }
    buf.push(0);
}

fn read_name(msg: &[u8], mut pos: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut jumped = false;
    let mut end = pos;
    let mut hops = 0;
    loop {
        if hops > 16 {
            return None;
        }
        let len = *msg.get(pos)? as usize;
        if len == 0 {
            if !jumped {
                end = pos + 1;
            }
            break;
        }
        if len & 0xC0 == 0xC0 {
            let b2 = *msg.get(pos + 1)? as usize;
            let ptr = ((len & 0x3F) << 8) | b2;
            if !jumped {
                end = pos + 2;
            }
            pos = ptr;
            jumped = true;
            hops += 1;
            continue;
        }
        if len & 0xC0 != 0 {
            return None;
        }
        pos += 1;
        let s = std::str::from_utf8(msg.get(pos..pos + len)?).ok()?;
        labels.push(s.to_owned());
        pos += len;
        if !jumped {
            end = pos;
        }
    }
    Some((labels.join("."), end))
}

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.id.to_be_bytes());
        buf.extend_from_slice(&self.flags.to_be_bytes());
        buf.extend_from_slice(&(self.questions.len() as u16).to_be_bytes());
        buf.extend_from_slice(&(self.answers.len() as u16).to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&(self.additionals.len() as u16).to_be_bytes());
        for q in &self.questions {
            write_name(&mut buf, &q.name);
            buf.extend_from_slice(&q.qtype.to_be_bytes());
            buf.extend_from_slice(&CLASS_IN.to_be_bytes());
        }
        for r in self.answers.iter().chain(self.additionals.iter()) {
            write_record(&mut buf, r);
        }
        buf
    }

    pub fn decode(msg: &[u8]) -> Option<Self> {
        if msg.len() < 12 {
            return None;
        }
        let id = u16::from_be_bytes(msg[0..2].try_into().ok()?);
        let flags = u16::from_be_bytes(msg[2..4].try_into().ok()?);
        let nq = u16::from_be_bytes(msg[4..6].try_into().ok()?) as usize;
        let na = u16::from_be_bytes(msg[6..8].try_into().ok()?) as usize;
        let nn = u16::from_be_bytes(msg[8..10].try_into().ok()?) as usize;
        let nad = u16::from_be_bytes(msg[10..12].try_into().ok()?) as usize;
        let mut pos = 12usize;
        let mut questions = Vec::new();
        for _ in 0..nq {
            let (name, p) = read_name(msg, pos)?;
            pos = p;
            let qtype = u16::from_be_bytes(msg.get(pos..pos + 2)?.try_into().ok()?);
            pos += 4; // type + class
            questions.push(Question { name, qtype });
        }
        let mut answers = Vec::new();
        for _ in 0..na {
            let (r, p) = read_record(msg, pos)?;
            pos = p;
            answers.push(r);
        }
        for _ in 0..nn {
            let (_, p) = read_record(msg, pos)?;
            pos = p;
        }
        let mut additionals = Vec::new();
        for _ in 0..nad {
            let (r, p) = read_record(msg, pos)?;
            pos = p;
            additionals.push(r);
        }
        Some(Message {
            id,
            flags,
            questions,
            answers,
            additionals,
        })
    }
}

fn write_record(buf: &mut Vec<u8>, r: &Record) {
    write_name(buf, &r.name);
    let rtype = match &r.data {
        RecordData::A(_) => QTYPE_A,
        RecordData::Ptr(_) => QTYPE_PTR,
        RecordData::Txt(_) => QTYPE_TXT,
        RecordData::Srv { .. } => QTYPE_SRV,
    };
    buf.extend_from_slice(&rtype.to_be_bytes());
    buf.extend_from_slice(&r.class.to_be_bytes());
    buf.extend_from_slice(&r.ttl.to_be_bytes());
    let rdata_len_at = buf.len();
    buf.extend_from_slice(&0u16.to_be_bytes());
    let start = buf.len();
    match &r.data {
        RecordData::A(ip) => buf.extend_from_slice(&ip.octets()),
        RecordData::Ptr(n) => write_name(buf, n),
        RecordData::Txt(strs) => {
            for s in strs {
                buf.push(s.len() as u8);
                buf.extend_from_slice(s);
            }
        }
        RecordData::Srv {
            priority,
            weight,
            port,
            target,
        } => {
            buf.extend_from_slice(&priority.to_be_bytes());
            buf.extend_from_slice(&weight.to_be_bytes());
            buf.extend_from_slice(&port.to_be_bytes());
            write_name(buf, target);
        }
    }
    let len = (buf.len() - start) as u16;
    buf[rdata_len_at..rdata_len_at + 2].copy_from_slice(&len.to_be_bytes());
}

fn read_record(msg: &[u8], pos: usize) -> Option<(Record, usize)> {
    let (name, mut pos) = read_name(msg, pos)?;
    let rtype = u16::from_be_bytes(msg.get(pos..pos + 2)?.try_into().ok()?);
    pos += 2;
    let class = u16::from_be_bytes(msg.get(pos..pos + 2)?.try_into().ok()?);
    pos += 2;
    let ttl = u32::from_be_bytes(msg.get(pos..pos + 4)?.try_into().ok()?);
    pos += 4;
    let rdlen = u16::from_be_bytes(msg.get(pos..pos + 2)?.try_into().ok()?) as usize;
    pos += 2;
    let rdata = msg.get(pos..pos + rdlen)?;
    let data = match rtype {
        QTYPE_A if rdata.len() == 4 => {
            RecordData::A(Ipv4Addr::new(rdata[0], rdata[1], rdata[2], rdata[3]))
        }
        QTYPE_PTR => {
            let (n, _) = read_name(msg, pos)?;
            RecordData::Ptr(n)
        }
        QTYPE_TXT => {
            let mut strs = Vec::new();
            let mut i = 0;
            while i < rdata.len() {
                let n = rdata[i] as usize;
                i += 1;
                if i + n > rdata.len() {
                    break;
                }
                strs.push(rdata[i..i + n].to_vec());
                i += n;
            }
            RecordData::Txt(strs)
        }
        QTYPE_SRV if rdata.len() >= 6 => {
            let priority = u16::from_be_bytes(rdata[0..2].try_into().ok()?);
            let weight = u16::from_be_bytes(rdata[2..4].try_into().ok()?);
            let port = u16::from_be_bytes(rdata[4..6].try_into().ok()?);
            let (target, _) = read_name(msg, pos + 6)?;
            RecordData::Srv {
                priority,
                weight,
                port,
                target,
            }
        }
        _ => return None,
    };
    Some((
        Record {
            name,
            rtype,
            class,
            ttl,
            data,
        },
        pos + rdlen,
    ))
}

pub fn service_instance(hostname: &str, service: &str) -> String {
    format!("{hostname}.{service}")
}

pub const ARC_SERVICE: &str = "_netaudio-arc._udp.local";
pub const CMC_SERVICE: &str = "_netaudio-cmc._udp.local";
pub const CHAN_SERVICE: &str = "_netaudio-chan._udp.local";

pub fn txt_kv(k: &str, v: &str) -> Vec<u8> {
    format!("{k}={v}").into_bytes()
}

#[allow(clippy::too_many_arguments)]
pub fn build_announcement(
    hostname: &str,
    ip: Ipv4Addr,
    arc_port: u16,
    cmc_port: u16,
    device_id_hex: &str,
    process_id: u16,
    board: &str,
    manufacturer: &str,
    model: &str,
) -> Message {
    let host_local = format!("{hostname}.local");
    let arc_inst = service_instance(hostname, ARC_SERVICE);
    let cmc_inst = service_instance(hostname, CMC_SERVICE);
    let arc_txt = vec![
        txt_kv("arcp_vers", "2.7.41"),
        txt_kv("arcp_min", "0.2.4"),
        txt_kv("router_vers", "4.0.2"),
        txt_kv("router_info", board),
        txt_kv("mf", manufacturer),
        txt_kv("model", model),
    ];
    let mut cmc_txt = vec![
        txt_kv("id", device_id_hex),
        txt_kv("process", &process_id.to_string()),
        txt_kv("cmcp_vers", "1.2.0"),
        txt_kv("cmcp_min", "1.0.0"),
        txt_kv("server_vers", "4.0.2"),
        txt_kv("channels", "0x6000004d"),
        txt_kv("mf", manufacturer),
        txt_kv("model", model),
        Vec::new(),
        Vec::new(),
    ];
    let _ = &mut cmc_txt;
    Message {
        id: 0,
        flags: FLAGS_RESPONSE,
        questions: Vec::new(),
        answers: vec![
            Record {
                name: ARC_SERVICE.into(),
                rtype: QTYPE_PTR,
                class: CLASS_IN,
                ttl: TTL_HOST,
                data: RecordData::Ptr(arc_inst.clone()),
            },
            Record {
                name: CMC_SERVICE.into(),
                rtype: QTYPE_PTR,
                class: CLASS_IN,
                ttl: TTL_HOST,
                data: RecordData::Ptr(cmc_inst.clone()),
            },
        ],
        additionals: vec![
            Record {
                name: arc_inst,
                rtype: QTYPE_SRV,
                class: CLASS_FLUSH,
                ttl: TTL_HOST,
                data: RecordData::Srv {
                    priority: 0,
                    weight: 0,
                    port: arc_port,
                    target: host_local.clone(),
                },
            },
            Record {
                name: service_instance(hostname, ARC_SERVICE),
                rtype: QTYPE_TXT,
                class: CLASS_FLUSH,
                ttl: TTL_HOST,
                data: RecordData::Txt(arc_txt),
            },
            Record {
                name: cmc_inst.clone(),
                rtype: QTYPE_SRV,
                class: CLASS_FLUSH,
                ttl: TTL_HOST,
                data: RecordData::Srv {
                    priority: 0,
                    weight: 0,
                    port: cmc_port,
                    target: host_local.clone(),
                },
            },
            Record {
                name: cmc_inst,
                rtype: QTYPE_TXT,
                class: CLASS_FLUSH,
                ttl: TTL_HOST,
                data: RecordData::Txt(cmc_txt),
            },
            Record {
                name: host_local,
                rtype: QTYPE_A,
                class: CLASS_FLUSH,
                ttl: TTL_HOST,
                data: RecordData::A(ip),
            },
        ],
    }
}

pub fn build_probe(hostname: &str) -> Message {
    let host_local = format!("{hostname}.local");
    let arc_inst = service_instance(hostname, ARC_SERVICE);
    let cmc_inst = service_instance(hostname, CMC_SERVICE);
    Message {
        id: 0,
        flags: FLAGS_QUERY,
        questions: vec![
            Question {
                name: host_local,
                qtype: QTYPE_ANY,
            },
            Question {
                name: arc_inst,
                qtype: QTYPE_ANY,
            },
            Question {
                name: cmc_inst,
                qtype: QTYPE_ANY,
            },
        ],
        answers: Vec::new(),
        additionals: Vec::new(),
    }
}

pub fn txt_get<'a>(strs: &'a [Vec<u8>], key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    for s in strs {
        if let Ok(t) = std::str::from_utf8(s) {
            if let Some(v) = t.strip_prefix(&prefix) {
                return Some(v);
            }
        }
    }
    None
}

pub fn chan_instance(tx_channel: &str, tx_hostname: &str) -> String {
    format!("{tx_channel}@{tx_hostname}.{CHAN_SERVICE}")
}

pub fn parse_fpp(v: &str) -> (u16, u16) {
    let mut parts = v.split(',');
    let max = parts
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(16);
    let min = parts
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(2);
    (max, min)
}

pub fn names_match(a: &str, b: &str) -> bool {
    a.trim_end_matches('.')
        .eq_ignore_ascii_case(b.trim_end_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announce_roundtrip_has_empty_txt() {
        let m = build_announcement(
            "netaudio",
            Ipv4Addr::new(192, 168, 1, 10),
            4440,
            8800,
            "0001020304050607",
            1,
            "netaudio-rs",
            "Mikansei",
            "netaudio",
        );
        let bytes = m.encode();
        let d = Message::decode(&bytes).unwrap();
        let txt = d
            .additionals
            .iter()
            .find(|r| matches!(r.data, RecordData::Txt(_)) && r.name.contains("cmc"))
            .unwrap();
        if let RecordData::Txt(s) = &txt.data {
            assert_eq!(s.iter().filter(|x| x.is_empty()).count(), 2);
        } else {
            panic!("expected txt");
        }
    }
}
