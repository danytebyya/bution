//! Zero-configuration peer discovery over mDNS/Bonjour.

use crate::PROTOCOL_VERSION;
use anyhow::{Context, Result};
use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent, ServiceInfo};
use std::net::IpAddr;
use uuid::Uuid;

pub const SERVICE_TYPE: &str = "_bution._tcp.local.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryAdvertisement {
    pub id: Uuid,
    pub name: String,
    pub public_key: String,
    pub role: String,
    pub backend: String,
    pub control_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredNode {
    pub id: Uuid,
    pub name: String,
    pub public_key: String,
    pub role: String,
    pub backend: String,
    pub addresses: Vec<IpAddr>,
    pub control_port: u16,
    pub protocol_version: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    Found(DiscoveredNode),
    Removed { fullname: String },
}

pub struct MdnsDiscovery {
    daemon: ServiceDaemon,
    receiver: Receiver<ServiceEvent>,
    local_id: Uuid,
    service_fullname: String,
}

impl MdnsDiscovery {
    pub fn start(advertisement: DiscoveryAdvertisement) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("could not start the mDNS service")?;
        let hostname = format!("bution-{}.local.", advertisement.id.simple());
        let id = advertisement.id.to_string();
        let protocol = PROTOCOL_VERSION.to_string();
        let port = advertisement.control_port.to_string();
        let properties = [
            ("id", id.as_str()),
            ("name", advertisement.name.as_str()),
            ("key", advertisement.public_key.as_str()),
            ("role", advertisement.role.as_str()),
            ("backend", advertisement.backend.as_str()),
            ("protocol", protocol.as_str()),
            ("control_port", port.as_str()),
        ];
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &id,
            &hostname,
            "",
            advertisement.control_port,
            &properties[..],
        )
        .context("could not create mDNS advertisement")?
        .enable_addr_auto();
        let service_fullname = service.get_fullname().to_owned();
        daemon
            .register(service)
            .context("could not publish BUTION on the local network")?;
        let receiver = daemon
            .browse(SERVICE_TYPE)
            .context("could not browse for BUTION nodes")?;

        Ok(Self {
            daemon,
            receiver,
            local_id: advertisement.id,
            service_fullname,
        })
    }

    pub async fn next(&self) -> Result<DiscoveryEvent> {
        loop {
            match self
                .receiver
                .recv_async()
                .await
                .context("mDNS discovery stopped")?
            {
                ServiceEvent::ServiceResolved(info) => {
                    let node = parse_service_info(&info)?;
                    if node.id != self.local_id {
                        return Ok(DiscoveryEvent::Found(node));
                    }
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    if fullname != self.service_fullname {
                        return Ok(DiscoveryEvent::Removed { fullname });
                    }
                }
                _ => {}
            }
        }
    }
}

impl Drop for MdnsDiscovery {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        let _ = self.daemon.unregister(&self.service_fullname);
        let _ = self.daemon.shutdown();
    }
}

fn parse_service_info(info: &ServiceInfo) -> Result<DiscoveredNode> {
    let property = |name: &str| {
        info.get_property_val_str(name)
            .map(str::to_owned)
            .with_context(|| format!("mDNS record is missing {name}"))
    };
    let id = property("id")?
        .parse()
        .context("mDNS record has an invalid node UUID")?;
    let protocol_version = property("protocol")?
        .parse()
        .context("mDNS record has an invalid protocol version")?;
    let control_port = property("control_port")?
        .parse()
        .context("mDNS record has an invalid control port")?;

    let mut addresses: Vec<_> = info.get_addresses().iter().copied().collect();
    addresses.sort_by_key(|address| (address.is_ipv6(), address.to_string()));
    Ok(DiscoveredNode {
        id,
        name: property("name")?,
        public_key: property("key")?,
        role: property("role")?,
        backend: property("backend")?,
        addresses,
        control_port,
        protocol_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resolved_mdns_record() {
        let id = Uuid::new_v4();
        let id_text = id.to_string();
        let properties = [
            ("id", id_text.as_str()),
            ("name", "HONOR Laptop"),
            ("key", "public-key"),
            ("role", "automatic"),
            ("backend", "CPU"),
            ("protocol", "1"),
            ("control_port", "31750"),
        ];
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &id_text,
            "honor.local.",
            "192.168.1.18",
            31_750,
            &properties[..],
        )
        .unwrap();
        let node = parse_service_info(&info).unwrap();
        assert_eq!(node.id, id);
        assert_eq!(node.name, "HONOR Laptop");
        assert_eq!(node.addresses[0].to_string(), "192.168.1.18");
    }
}
