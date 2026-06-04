use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{mint_to_checked, Mint, MintToChecked, TokenAccount, TokenInterface},
};
use mpl_core::{
    accounts::{BaseAssetV1, BaseCollectionV1},
    instructions::UpdatePluginV1CpiBuilder,
    types::{Attribute, Attributes, Plugin, UpdateAuthority},
    ID as MPL_CORE_ID,
};

use crate::{error::ErrorCode, state::Config, utils::load_asset_attributes};

const SECONDS_PER_DAY: i64 = 86400;

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [b"config", collection.key().as_ref()],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        has_one = owner @ ErrorCode::InvalidOwner,
        constraint = asset.update_authority == UpdateAuthority::Collection(collection.key()) @ ErrorCode::InvalidUpdateAuthority,
    )]
    pub asset: Account<'info, BaseAssetV1>,

    #[account(
        mut,
        has_one = update_authority @ ErrorCode::InvalidUpdateAuthority
    )]
    pub collection: Account<'info, BaseCollectionV1>,

    /// CHECK: This account data is not used, we only verify the address
    #[account(
        seeds = [b"update_authority", collection.key().as_ref()],
        bump
    )]
    pub update_authority: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [b"rewards_mint", config.key().as_ref()],
        bump = config.rewards_bump,
    )]
    pub rewards_mint: InterfaceAccount<'info, Mint>,

    #[account(
        init_if_needed,
        payer = owner,
        associated_token::mint = rewards_mint,
        associated_token::authority = owner
    )]
    pub user_rewards_ata: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,

    pub associated_token_program: Program<'info, AssociatedToken>,

    pub system_program: Program<'info, System>,

    /// CHECK: This is the ID of the MPL Core Program
    #[account(address = MPL_CORE_ID)]
    pub mpl_core_program: UncheckedAccount<'info>,
}

pub fn handler(ctx: Context<Claim>) -> Result<()> {
    let attributes = load_asset_attributes(&ctx.accounts.asset.to_account_info())?
        .ok_or(ErrorCode::AssetNotStaked)?;

    let current_timestamp = Clock::get()?.unix_timestamp;
    let mut staked_at: Option<i64> = None;
    let mut rewards_updated_at: Option<i64> = None;
    let mut is_staked = false;
    let mut attributes_list = Vec::with_capacity(attributes.attribute_list.len());

    for attribute in &attributes.attribute_list {
        match attribute.key.as_str() {
            "staked" => {
                require!(attribute.value == "true", ErrorCode::AssetNotStaked);
                is_staked = true;
                attributes_list.push(attribute.clone());
            }
            "staked_at" => {
                staked_at = Some(
                    attribute
                        .value
                        .parse::<i64>()
                        .map_err(|_| ErrorCode::InvalidTimestamp)?,
                );
                attributes_list.push(attribute.clone());
            }
            "rewards_updated_at" => {
                rewards_updated_at = Some(
                    attribute
                        .value
                        .parse::<i64>()
                        .map_err(|_| ErrorCode::InvalidTimestamp)?,
                );
                attributes_list.push(Attribute {
                    key: "rewards_updated_at".to_string(),
                    value: current_timestamp.to_string(),
                });
            }
            _ => attributes_list.push(attribute.clone()),
        }
    }

    require!(is_staked, ErrorCode::AssetNotStaked);
    let rewards_started_at = rewards_updated_at.or(staked_at).ok_or(ErrorCode::InvalidTimestamp)?;

    if rewards_updated_at.is_none() {
        attributes_list.push(Attribute {
            key: "rewards_updated_at".to_string(),
            value: current_timestamp.to_string(),
        });
    }

    let staked_time = current_timestamp
        .checked_sub(rewards_started_at)
        .ok_or(ErrorCode::InvalidTimestamp)?
        .checked_div(SECONDS_PER_DAY)
        .ok_or(ErrorCode::InvalidTimestamp)?;

    require!(staked_time > 0, ErrorCode::NoRewardsToClaim);

    let collection_key = ctx.accounts.collection.key();
    let signer_seeds = &[
        b"update_authority",
        collection_key.as_ref(),
        &[ctx.bumps.update_authority],
    ];

    UpdatePluginV1CpiBuilder::new(&ctx.accounts.mpl_core_program.to_account_info())
    .asset(&ctx.accounts.asset.to_account_info())
    .collection(Some(&ctx.accounts.collection.to_account_info()))
    .payer(&ctx.accounts.owner.to_account_info())
    .authority(Some(&ctx.accounts.update_authority.to_account_info()))
    .system_program(&ctx.accounts.system_program.to_account_info())
    .plugin(Plugin::Attributes(Attributes { attribute_list: attributes_list }))
    .invoke_signed(&[signer_seeds])?;

    let amount = (staked_time as u64)
        .checked_mul(ctx.accounts.config.rewards_bps as u64)
        .ok_or(ErrorCode::InvalidRewardsBps)?
        .checked_mul(10u64.pow(ctx.accounts.rewards_mint.decimals as u32))
        .ok_or(ErrorCode::InvalidRewardsBps)?
        .checked_div(10000u64)
        .ok_or(ErrorCode::InvalidRewardsBps)?;

    let config_seeds = &[
        b"config",
        collection_key.as_ref(),
        &[ctx.accounts.config.bump],
    ];

    let config_signer_seeds: &[&[&[u8]]; 1] = &[&config_seeds[..]];

    if amount > 0 {
        mint_to_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintToChecked {
                    mint: ctx.accounts.rewards_mint.to_account_info(),
                    to: ctx.accounts.user_rewards_ata.to_account_info(),
                    authority: ctx.accounts.config.to_account_info(),
                },
                config_signer_seeds,
            ),
            amount,
            ctx.accounts.rewards_mint.decimals,
        )?;
    }

    Ok(())
}
