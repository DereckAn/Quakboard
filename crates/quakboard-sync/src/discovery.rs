//! Finding other Quakboard devices on the LAN with mDNS.
//!
//! Each device advertises `_quakboard._tcp.local.` while sync is on. The
//! instance name, host name and TXT record carry only the opaque device id, so
//! people on the same Wi-Fi can't see a hostname or user name. Friendly names
//! are exchanged later, encrypted, during pairing.
//!
//! The mDNS daemon is behind the `mdns` feature. Without it (the phone apps,
//! which use native Bonjour/NSD) only the names both sides must agree on are
//! here.

use std::net::SocketAddr;
#[cfg(feature = "mdns")]
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
};

#[cfg(feature = "mdns")]
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;

pub const SERVICE_TYPE: &str = "_quakboard._tcp.local.";
/// TXT key carrying the device id; the instance name is the id too.
pub const ID_PROPERTY: &str = "id";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredDevice {
    pub id: String,
    pub addr: SocketAddr,
}

/// Found devices by mDNS full name, which is what removal events carry.
#[cfg(feature = "mdns")]
type Devices = Arc<Mutex<HashMap<String, DiscoveredDevice>>>;

/// Advertises this device and tracks the others. Dropping it stops both.
#[cfg(feature = "mdns")]
pub struct Discovery {
    daemon: ServiceDaemon,
    devices: Devices,
}

#[cfg(feature = "mdns")]
impl Discovery {
    pub fn start(device_id: &str, port: u16) -> Result<Self, mdns_sd::Error> {
        let daemon = ServiceDaemon::new()?;

        let properties = [(ID_PROPERTY, device_id)];
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            device_id,
            &format!("{device_id}.local."),
            "",
            port,
            &properties[..],
        )?
        // Advertise every current interface address, following changes.
        .enable_addr_auto();
        daemon.register(service)?;

        let devices = Devices::default();
        let events = daemon.browse(SERVICE_TYPE)?;
        let tracked = Arc::clone(&devices);
        let own_id = device_id.to_string();
        // Ends when the daemon shuts down and closes the channel.
        std::thread::spawn(move || {
            while let Ok(event) = events.recv() {
                track(&tracked, &own_id, event);
            }
        });

        Ok(Discovery { daemon, devices })
    }

    /// Devices currently visible, sorted by id so the list doesn't jump around.
    pub fn devices(&self) -> Vec<DiscoveredDevice> {
        let mut devices: Vec<_> = lock(&self.devices).values().cloned().collect();
        devices.sort_by(|a, b| a.id.cmp(&b.id));
        devices
    }

    pub fn address_of(&self, device_id: &str) -> Option<SocketAddr> {
        lock(&self.devices)
            .values()
            .find(|device| device.id == device_id)
            .map(|device| device.addr)
    }
}

#[cfg(feature = "mdns")]
impl Drop for Discovery {
    fn drop(&mut self) {
        // Also sends goodbye packets, so others drop us right away.
        if let Err(e) = self.daemon.shutdown() {
            eprintln!("Failed to stop mDNS discovery: {e}");
        }
    }
}

/// Apply one browse event. Everything in it came off the network.
#[cfg(feature = "mdns")]
fn track(devices: &Mutex<HashMap<String, DiscoveredDevice>>, own_id: &str, event: ServiceEvent) {
    match event {
        ServiceEvent::ServiceResolved(service) => {
            let Some(id) = service.get_property_val_str(ID_PROPERTY) else {
                return;
            };
            if id == own_id || uuid::Uuid::parse_str(id).is_err() {
                return;
            }
            // ponytail: IPv4 only. IPv6 link-local addresses need the interface
            // scope to connect; add them if IPv4-less LANs show up.
            let Some(ip) = pick_address(service.get_addresses_v4()) else {
                return;
            };
            let device = DiscoveredDevice {
                id: id.to_string(),
                addr: SocketAddr::new(IpAddr::V4(ip), service.port),
            };
            println!("Sync found device #{} at {}", short_id(id), device.addr);
            lock(devices).insert(service.fullname.clone(), device);
        }
        ServiceEvent::ServiceRemoved(_, fullname) => {
            if let Some(device) = lock(devices).remove(&fullname) {
                println!("Sync lost device #{}", short_id(&device.id));
            }
        }
        _ => {}
    }
}

/// Prefer a real network address: a device can advertise 127.0.0.1 too, and
/// that only reaches it from the same machine. Lowest wins, for stability.
#[cfg(feature = "mdns")]
fn pick_address(addresses: impl IntoIterator<Item = Ipv4Addr>) -> Option<Ipv4Addr> {
    let (loopback, lan): (Vec<_>, Vec<_>) = addresses.into_iter().partition(Ipv4Addr::is_loopback);
    lan.into_iter().min().or_else(|| loopback.into_iter().min())
}

/// The "#ab12" suffix shown for devices that aren't paired yet.
pub fn short_id(device_id: &str) -> &str {
    let start = device_id.len().saturating_sub(4);
    device_id.get(start..).unwrap_or(device_id)
}

#[cfg(feature = "mdns")]
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(all(test, feature = "mdns"))]
mod tests {
    use super::*;

    const OWN_ID: &str = "11111111-1111-4111-8111-111111111111";
    const OTHER_ID: &str = "22222222-2222-4222-8222-2222222222ab";

    fn resolved(id: &str, ip: &str) -> ServiceEvent {
        let properties = [(ID_PROPERTY, id)];
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            id,
            &format!("{id}.local."),
            ip,
            4000,
            &properties[..],
        )
        .unwrap();
        ServiceEvent::ServiceResolved(Box::new(info.as_resolved_service()))
    }

    fn tracked(events: Vec<ServiceEvent>) -> Vec<DiscoveredDevice> {
        let devices = Mutex::new(HashMap::new());
        for event in events {
            track(&devices, OWN_ID, event);
        }
        let found = lock(&devices).values().cloned().collect();
        found
    }

    #[test]
    fn resolved_device_is_listed_with_its_address() {
        assert_eq!(
            tracked(vec![resolved(OTHER_ID, "192.168.1.20")]),
            vec![DiscoveredDevice {
                id: OTHER_ID.into(),
                addr: "192.168.1.20:4000".parse().unwrap(),
            }]
        );
    }

    #[test]
    fn own_advertisement_is_ignored() {
        assert!(tracked(vec![resolved(OWN_ID, "192.168.1.10")]).is_empty());
    }

    #[test]
    fn non_uuid_id_is_ignored() {
        assert!(tracked(vec![resolved("not-a-uuid", "192.168.1.20")]).is_empty());
    }

    #[test]
    fn ipv6_only_device_is_ignored() {
        assert!(tracked(vec![resolved(OTHER_ID, "fe80::1")]).is_empty());
    }

    #[test]
    fn removed_device_is_dropped() {
        let fullname = format!("{OTHER_ID}.{SERVICE_TYPE}");
        let removed = ServiceEvent::ServiceRemoved(SERVICE_TYPE.into(), fullname);
        assert!(tracked(vec![resolved(OTHER_ID, "192.168.1.20"), removed]).is_empty());
    }

    #[test]
    fn re_resolving_a_device_updates_its_address() {
        let devices = tracked(vec![
            resolved(OTHER_ID, "192.168.1.20"),
            resolved(OTHER_ID, "192.168.1.99"),
        ]);
        assert_eq!(devices[0].addr, "192.168.1.99:4000".parse().unwrap());
    }

    #[test]
    fn lan_address_is_preferred_over_loopback() {
        let picked = pick_address([
            "127.0.0.1".parse().unwrap(),
            "192.168.18.244".parse().unwrap(),
        ]);
        assert_eq!(picked, Some("192.168.18.244".parse().unwrap()));
    }

    #[test]
    fn loopback_is_used_when_it_is_the_only_address() {
        let picked = pick_address(["127.0.0.1".parse().unwrap()]);
        assert_eq!(picked, Some(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn short_id_is_the_last_four_characters() {
        assert_eq!(short_id(OTHER_ID), "22ab");
    }
}
