use crate::{
    context::Context, dht::actions::remove_link::remove_link, network::chain_pair::ChainPair,
    nucleus::validation::validate_entry, workflows::hold_entry::hold_entry_workflow,
};

use crate::{nucleus::validation::ValidationError, workflows::validation_package};
use holochain_core_types::{
    entry::Entry,
    error::HolochainError,
    validation::{EntryLifecycle, ValidationData},
};
use std::sync::Arc;

pub async fn remove_link_workflow(
    chain_pair: &ChainPair,
    context: Arc<Context>,
) -> Result<(), HolochainError> {
    let link_remove = match &chain_pair.entry() {
        Entry::LinkRemove((link_remove, _)) => link_remove,
        _ => Err(HolochainError::ErrorGeneric(
            "remove_link_workflow expects entry to be an Entry::LinkRemove".to_string(),
        ))?,
    };
    let link = link_remove.link().clone();

    log_debug!(context, "workflow/remove_link: {:?}", link);
    // 1. Get hold of validation package
    log_debug!(
        context,
        "workflow/remove_link: getting validation package..."
    );
    let maybe_validation_package = validation_package(&chain_pair, context.clone())
        .await
        .map_err(|err| {
            let message = "Could not get validation package from source! -> Add to pending...";
            log_debug!(context, "workflow/remove_link: {}", message);
            log_debug!(context, "workflow/remove_link: Error was: {:?}", err);
            HolochainError::ValidationPending
        })?;

    let validation_package = maybe_validation_package
        .ok_or_else(|| "Could not get validation package from source".to_string())?;
    log_debug!(context, "workflow/remove_link: got validation package!");

    // 2. Create validation data struct
    let validation_data = ValidationData {
        package: validation_package,
        lifecycle: EntryLifecycle::Meta,
    };

    // 3. Validate the entry
    log_debug!(context, "workflow/remove_link: validate...");
    validate_entry(
        chain_pair.entry().clone(),
        None,
        validation_data,
        &context
    ).await
    .map_err(|err| {
        if let ValidationError::UnresolvedDependencies(dependencies) = &err {
            log_debug!(context, "workflow/remove_link: Link could not be validated due to unresolved dependencies and will be tried later. List of missing dependencies: {:?}", dependencies);
            HolochainError::ValidationPending
        } else {
            log_warn!(context, "workflow/remove_link: Link {:?} is NOT valid! Validation error: {:?}",
                chain_pair.entry(),
                err,
            );
            HolochainError::from(err)
        }

    })?;

    log_debug!(context, "workflow/remove_link: is valid!");

    // 3. If valid store remove the entry in the local DHT shard
    remove_link(&chain_pair.entry(), &context).await?;
    log_debug!(context, "workflow/remove_link: added! {:?}", link);

    //4. store link_remove entry so we have all we need to respond to get links queries without any other network look-up```
    hold_entry_workflow(&chain_pair, context.clone()).await?;
    log_debug!(context, "workflow/hold_entry: added! {:?}", chain_pair);

    Ok(())
}
