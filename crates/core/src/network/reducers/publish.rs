use crate::{
    action::ActionWrapper,
    network::{
        actions::NetworkActionResponse, entry_header_pair::EntryHeaderPair, entry_aspect::EntryAspect,
        reducers::send, state::NetworkState,
    },
    state::State,
};
use chrono::{offset::FixedOffset, DateTime};
use holochain_core_types::{
    crud_status::CrudStatus,
    entry::{entry_type::EntryType, Entry},
    error::HolochainError,
};
use holochain_json_api::json::JsonString;
use lib3h_protocol::{
    data_types::{EntryAspectData, EntryData, ProvidedEntryData},
    protocol_client::Lib3hClientProtocol,
};

use crate::network::actions::Response;
use holochain_persistence_api::cas::content::{Address, AddressableContent};

pub fn entry_data_to_entry_aspect_data(ea: &EntryAspect) -> EntryAspectData {
    let type_hint = ea.type_hint();
    let aspect_address = ea.address();
    let ts: DateTime<FixedOffset> = ea.header().timestamp().into();
    let aspect_json: JsonString = ea.into();
    EntryAspectData {
        type_hint,
        aspect_address: aspect_address.into(),
        aspect: aspect_json.to_bytes().into(),
        publish_ts: ts.timestamp() as u64,
    }
}

/// Send to network a PublishDhtData message
fn publish_entry(
    network_state: &mut NetworkState,
    entry_header_pair: &EntryHeaderPair,
) -> Result<(), HolochainError> {
    send(
        network_state,
        Lib3hClientProtocol::PublishEntry(ProvidedEntryData {
            space_address: network_state.dna_address.clone().unwrap().into(),
            provider_agent_id: network_state.agent_id.clone().unwrap().into(),
            entry: EntryData {
                entry_address: entry_header_pair.entry().address().into(),
                aspect_list: vec![entry_data_to_entry_aspect_data(&EntryAspect::Content(
                    entry_header_pair.entry(),
                    entry_header_pair.header(),
                ))],
            },
        }),
    )
}

/// Send to network a publish request for either delete or update aspect information
fn publish_update_delete_meta(
    network_state: &mut NetworkState,
    orig_entry_address: Address,
    crud_status: CrudStatus,
    entry_header_pair: &EntryHeaderPair,
) -> Result<(), HolochainError> {
    // publish crud-status

    let aspect = match crud_status {
        CrudStatus::Modified => EntryAspect::Update(entry_header_pair.entry(), entry_header_pair.header()),
        CrudStatus::Deleted => EntryAspect::Deletion(entry_header_pair.header()),
        crud => {
            return Err(HolochainError::ErrorGeneric(format!(
                "Unexpeced CRUD variant {:?}",
                crud
            )));
        }
    };

    send(
        network_state,
        Lib3hClientProtocol::PublishEntry(ProvidedEntryData {
            space_address: network_state.dna_address.clone().unwrap().into(),
            provider_agent_id: network_state.agent_id.clone().unwrap().into(),
            entry: EntryData {
                entry_address: orig_entry_address.into(),
                aspect_list: vec![entry_data_to_entry_aspect_data(&aspect)],
            },
        }),
    )?;

    // publish crud-link if there is one
    Ok(())
}

/// Send to network a PublishMeta message holding a link metadata to `entry_header_pair`
fn publish_link_meta(
    network_state: &mut NetworkState,
    entry_header_pair: &EntryHeaderPair,
) -> Result<(), HolochainError> {
    let (base, aspect) = match entry_header_pair.entry() {
        Entry::LinkAdd(link_data) => (
            link_data.link().base().clone(),
            EntryAspect::LinkAdd(link_data, entry_header_pair.header()),
        ),
        Entry::LinkRemove((link_data, links_to_remove)) => (
            link_data.link().base().clone(),
            EntryAspect::LinkRemove((link_data, links_to_remove), entry_header_pair.header()),
        ),
        _ => {
            return Err(HolochainError::ErrorGeneric(format!(
                "Received bad entry type. Expected Entry::LinkAdd/Remove received {:?}",
                entry_header_pair.entry(),
            )));
        }
    };
    send(
        network_state,
        Lib3hClientProtocol::PublishEntry(ProvidedEntryData {
            space_address: network_state.dna_address.clone().unwrap().into(),
            provider_agent_id: network_state.agent_id.clone().unwrap().into(),
            entry: EntryData {
                entry_address: base.into(),
                aspect_list: vec![entry_data_to_entry_aspect_data(&aspect)],
            },
        }),
    )
}

fn reduce_publish_inner(
    network_state: &mut NetworkState,
    root_state: &State,
    address: &Address,
) -> Result<(), HolochainError> {
    network_state.initialized()?;

    let entry_header_pair = EntryHeaderPair::fetch_entry_header_pair(&address, root_state)?;

    match entry_header_pair.entry().entry_type() {
        EntryType::AgentId => publish_entry(network_state, &entry_header_pair),
        EntryType::App(_) => {
            publish_entry(network_state, &entry_header_pair).and_then(|_| {
                match entry_header_pair.header().link_update_delete() {
                    Some(modified_entry) => publish_update_delete_meta(
                        network_state,
                        modified_entry,
                        CrudStatus::Modified,
                        &entry_header_pair.clone(),
                    ),
                    None => Ok(()),
                }
            })
        }
        EntryType::LinkAdd => publish_entry(network_state, &entry_header_pair)
            .and_then(|_| publish_link_meta(network_state, &entry_header_pair)),
        EntryType::LinkRemove => publish_entry(network_state, &entry_header_pair)
            .and_then(|_| publish_link_meta(network_state, &entry_header_pair)),
        EntryType::Deletion => publish_entry(network_state, &entry_header_pair).and_then(|_| {
            match entry_header_pair.header().link_update_delete() {
                Some(modified_entry) => publish_update_delete_meta(
                    network_state,
                    modified_entry,
                    CrudStatus::Deleted,
                    &entry_header_pair.clone(),
                ),
                None => Ok(()),
            }
        }),
        _ => Err(HolochainError::NotImplemented(format!(
            "reduce_publish_inner not implemented for {}",
            entry_header_pair.entry().entry_type()
        ))),
    }
}

pub fn reduce_publish(
    network_state: &mut NetworkState,
    root_state: &State,
    action_wrapper: &ActionWrapper,
) {
    let action = action_wrapper.action();
    let address = unwrap_to!(action => crate::action::Action::Publish);

    let result = reduce_publish_inner(network_state, root_state, &address);
    network_state.actions.insert(
        action_wrapper.clone(),
        Response::from(NetworkActionResponse::Publish(match result {
            Ok(_) => Ok(address.clone()),
            Err(e) => Err(HolochainError::ErrorGeneric(e.to_string())),
        })),
    );
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::{
        action::{Action, ActionWrapper},
        instance::tests::test_context,
        state::test_store,
    };
    use chrono::{offset::FixedOffset, DateTime};
    use holochain_core_types::{chain_header::test_chain_header, entry::test_entry};
    use holochain_persistence_api::cas::content::AddressableContent;
    use lib3h_protocol::types::AspectHash;

    #[test]
    pub fn reduce_publish_test() {
        let context = test_context("alice", None);
        let store = test_store(context.clone());

        let entry = test_entry();
        let action_wrapper = ActionWrapper::new(Action::Publish(entry.address()));

        store.reduce(action_wrapper);
    }

    #[test]
    fn can_convert_into_entry_aspect_data() {
        let chain_header = test_chain_header();
        let aspect = EntryAspect::Header(chain_header.clone());
        let aspect_data: EntryAspectData = entry_data_to_entry_aspect_data(&aspect);
        let aspect_json: JsonString = aspect.clone().into();
        let ts: DateTime<FixedOffset> = chain_header.timestamp().into();
        assert_eq!(aspect_data.type_hint, aspect.type_hint());
        assert_eq!(
            aspect_data.aspect_address,
            AspectHash::from(aspect.address())
        );
        assert_eq!(*aspect_data.aspect, aspect_json.to_bytes());
        assert_eq!(aspect_data.publish_ts, ts.timestamp() as u64);
    }
}
