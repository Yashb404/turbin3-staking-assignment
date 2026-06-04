use anchor_lang::prelude::*;
use mpl_core::{
    accounts::{BaseAssetV1, PluginHeaderV1},
    types::{Attributes, Plugin, UpdateAuthority},
    PluginRegistryV1Safe,
};

fn asset_base_len(asset: &BaseAssetV1) -> usize {
    let mut size = 1 + 32 + 1 + 4 + 4 + 1 + asset.name.len() + asset.uri.len();

    if matches!(
        asset.update_authority,
        UpdateAuthority::Address(_) | UpdateAuthority::Collection(_)
    ) {
        size += 32;
    }

    if asset.seq.is_some() {
        size += 8;
    }

    size
}

pub fn load_asset_attributes(asset_info: &AccountInfo) -> Result<Option<Attributes>> {
    let data = asset_info.try_borrow_data()?;
    let asset = BaseAssetV1::from_bytes(&data).map_err(|_| error!(crate::error::ErrorCode::InvalidTimestamp))?;
    let base_len = asset_base_len(&asset);

    if base_len == data.len() {
        return Ok(None);
    }

    let plugin_header = PluginHeaderV1::from_bytes(&data[base_len..])
        .map_err(|_| error!(crate::error::ErrorCode::InvalidTimestamp))?;
    let plugin_registry = PluginRegistryV1Safe::from_bytes(&data[plugin_header.plugin_registry_offset as usize..])
        .map_err(|_| error!(crate::error::ErrorCode::InvalidTimestamp))?;

    for registry_record in plugin_registry.registry {
        let plugin = Plugin::deserialize(&mut &data[registry_record.offset as usize..])
            .map_err(|_| error!(crate::error::ErrorCode::InvalidTimestamp))?;

        if let Plugin::Attributes(attributes) = plugin {
            return Ok(Some(attributes));
        }
    }

    Ok(None)
}
