use anyhow::Ok;
use anyhow::{Result, anyhow};
use futures::stream::StreamExt;
use futures::stream::TryStreamExt;
use ipnetwork::Ipv4Network;
use log::{debug, info};
use netlink_packet_route::address::{AddressAttribute::Label, AddressMessage};
use netlink_packet_route::link::{LinkLayerType::Ether, LinkMessage};
use netlink_packet_route::rule::RuleAction::ToTable;
use netlink_sys::{AsyncSocket, SocketAddr};
use rtnetlink::{Handle, RouteMessageBuilder, constants::RTMGRP_IPV4_IFADDR, new_connection};
use std::collections::HashMap;
use std::net::IpAddr;
use std::{env::var, net::Ipv4Addr};

pub async fn start_route() -> Result<()> {
    let (mut connection, handle, mut messages) = new_connection()?;
    let mgroup_flags = RTMGRP_IPV4_IFADDR;
    let addr = SocketAddr::new(0, mgroup_flags);
    connection
        .socket_mut()
        .socket_mut()
        .bind(&addr)
        .expect("failed to bind");
    tokio::spawn(connection);

    let mut route_tables = HashMap::new();
    route_tables.insert("eth0".into(), 100u32);
    for i in 0..7 {
        route_tables.insert(format!("ppp{i}"), 101u32 + i);
    }

    for i in 0..8 {
        let table = 100 + i;
        let _ = add_rule(handle.clone(), table, format!("tun{i}"))
            .await
            .map_err(|x| debug!("add tun{i} rule failed: {x:?}"));
    }

    let oif = "eth0";
    let oif_info = get_link_by_name(handle.clone(), oif.into()).await?;
    let oif_idx = oif_info.header.index;
    let oif_type = oif_info.header.link_layer_type;
    let _ = add_default_route(handle.clone(), oif_idx, 100, oif_type == Ether)
        .await
        .map_err(|x| debug!("add {oif} route failed: {x:?}"));

    tokio::spawn(async move {
        while let Some((message, _)) = messages.next().await {
            if let netlink_packet_core::NetlinkPayload::InnerMessage(msg) = message.payload
                && let netlink_packet_route::RouteNetlinkMessage::NewAddress(route_msg) = msg
                && route_msg.header.prefix_len == 32
                && is_same_address(route_msg.attributes.iter()).await
            {
                let name = get_name(route_msg.clone()).await;
                let table = route_tables.get(&name).expect("why");
                let _ = add_default_route(handle.clone(), route_msg.header.index, *table, false)
                    .await
                    .map_err(|x| debug!("add {name} route failed: {x:?}"));
            }
        }
        panic!("no way...")
    });

    Ok(())
}

async fn add_rule(handle: Handle, table_id: u32, iif: String) -> Result<()> {
    handle
        .rule()
        .add()
        .v4()
        .action(ToTable)
        .input_interface(iif.clone())
        .table_id(table_id)
        .priority(table_id)
        .execute()
        .await?;
    info!("Rule added for {iif} to table {table_id}");
    Ok(())
}

async fn add_default_route(handle: Handle, idx: u32, table_id: u32, host: bool) -> Result<()> {
    let dest = Ipv4Network::new(Ipv4Addr::new(0, 0, 0, 0), 0).unwrap();

    let mut builder = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(dest.ip(), dest.prefix())
        .output_interface(idx)
        .table_id(table_id);

    if host {
        builder = builder.gateway(var("GATEWAY")?.parse()?)
    }
    let route = builder.build();
    handle.route().add(route).execute().await?;
    info!(
        "Default route added to table {table_id} via interface index {idx}{}",
        if host { " with gateway" } else { "" }
    );
    Ok(())
}

async fn get_link_by_name(handle: Handle, name: String) -> Result<LinkMessage> {
    let mut links = handle.link().get().match_name(name).execute();
    let link = links.try_next().await?;
    match link {
        Some(link) => {
            assert!(links.try_next().await?.is_none());
            Ok(link)
        }
        _ => Err(anyhow!("WHY?")),
    }
}

async fn get_name(msg: AddressMessage) -> String {
    for i in msg.attributes {
        if let Label(name) = i {
            return name;
        }
    }
    panic!("No name found");
}

async fn is_same_address(
    iter: std::slice::Iter<'_, netlink_packet_route::address::AddressAttribute>,
) -> bool {
    let mut addr: Option<IpAddr> = None;
    let mut local: Option<IpAddr> = None;

    for i in iter {
        match i {
            netlink_packet_route::address::AddressAttribute::Address(a) => {
                addr = Some(*a);
            }
            netlink_packet_route::address::AddressAttribute::Local(a) => {
                local = Some(*a);
            }
            _ => {}
        }
    }
    Some(addr) == Some(local)
}
