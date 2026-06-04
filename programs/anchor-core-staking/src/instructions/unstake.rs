use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{
        Mint,
        TokenAccount,
        TokenInterface,
        mint_to_checked,
        MintToChecked
    }
};
use mpl_core::{
    ID as MPL_CORE_ID,
    accounts::{BaseAssetV1, BaseCollectionV1},
    instructions::{UpdateCollectionPluginV1CpiBuilder, UpdatePluginV1CpiBuilder},
    types::{UpdateAuthority, Attribute, Attributes, Plugin, FreezeDelegate},
};
use crate::{error::ErrorCode, state::Config, utils::load_asset_attributes};

const SECONDS_PER_DAY: i64 = 86400;

#[derive(Accounts)]
pub struct Unstake<'info> {

    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
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

pub fn handler(ctx: Context<Unstake>) -> Result<()> {

    // We start by fetching the existing attributes
    let attributes_fetched = load_asset_attributes(&ctx.accounts.asset.to_account_info())?;

    require!(attributes_fetched.is_some(), ErrorCode::AssetNotStaked);

    let attributes = attributes_fetched.unwrap();

    let current_timestamp = Clock::get()?.unix_timestamp;
    let mut attributes_list = Vec::with_capacity(attributes.attribute_list.len());
    let mut staked_at: Option<i64> = None;
    let mut rewards_updated_at: Option<i64> = None;

    for attribute in &attributes.attribute_list {
        if attribute.key == "staked" {
            require!(attribute.value == "true", ErrorCode::AssetNotStaked);
        } else if attribute.key == "staked_at" {
            let parsed_staked_at = attribute
                .value
                .parse::<i64>()
                .map_err(|_| ErrorCode::InvalidTimestamp)?;
            staked_at = Some(parsed_staked_at);
            let freeze_time = current_timestamp
                .checked_sub(parsed_staked_at)
                .ok_or(ErrorCode::InvalidTimestamp)?
                .checked_div(SECONDS_PER_DAY)
                .ok_or(ErrorCode::InvalidTimestamp)?;
            require!(freeze_time >= ctx.accounts.config.freeze_period as i64, ErrorCode::FreezePeriodNotElapsed);
        } else if attribute.key == "rewards_updated_at" {
            rewards_updated_at = Some(
                attribute
                    .value
                    .parse::<i64>()
                    .map_err(|_| ErrorCode::InvalidTimestamp)?,
            );
        } else {
            attributes_list.push(attribute.clone());
        }
    }

    let rewards_started_at = rewards_updated_at.or(staked_at).ok_or(ErrorCode::InvalidTimestamp)?;

    let staked_time = current_timestamp
        .checked_sub(rewards_started_at)
        .ok_or(ErrorCode::InvalidTimestamp)?
        .checked_div(SECONDS_PER_DAY)
        .ok_or(ErrorCode::InvalidTimestamp)?;

    let collection_key = ctx.accounts.collection.key();
    let signer_seeds = &[
        b"update_authority",
        collection_key.as_ref(),
        &[ctx.bumps.update_authority],
    ];



    attributes_list.push(Attribute { 
        key: "staked".to_string(), 
        value: "false".to_string(), 
    });

    attributes_list.push(Attribute { 
        key: "staked_at".to_string(), 
        value: "0".to_string(), 
    });

    attributes_list.push(Attribute {
        key: "rewards_updated_at".to_string(),
        value: "0".to_string(),
    });

   
    UpdatePluginV1CpiBuilder::new(&ctx.accounts.mpl_core_program.to_account_info())
    .asset(&ctx.accounts.asset.to_account_info())
    .collection(Some(&ctx.accounts.collection.to_account_info()))
    .payer(&ctx.accounts.owner.to_account_info())
    .authority(Some(&ctx.accounts.owner.to_account_info()))
    .system_program(&ctx.accounts.system_program.to_account_info())
    .plugin(Plugin::FreezeDelegate(FreezeDelegate { frozen: false }))
    .invoke()?;

    UpdatePluginV1CpiBuilder::new(&ctx.accounts.mpl_core_program.to_account_info())
    .asset(&ctx.accounts.asset.to_account_info())
    .collection(Some(&ctx.accounts.collection.to_account_info()))
    .payer(&ctx.accounts.owner.to_account_info())
    .authority(Some(&ctx.accounts.update_authority.to_account_info()))
    .system_program(&ctx.accounts.system_program.to_account_info())
    .plugin(Plugin::Attributes(Attributes { attribute_list: attributes_list }))
    .invoke_signed(&[signer_seeds])?;

    ctx.accounts.config.staked_count = ctx
        .accounts
        .config
        .staked_count
        .checked_sub(1)
        .ok_or(ErrorCode::InvalidTimestamp)?;

    UpdateCollectionPluginV1CpiBuilder::new(&ctx.accounts.mpl_core_program.to_account_info())
    .collection(&ctx.accounts.collection.to_account_info())
    .payer(&ctx.accounts.owner.to_account_info())
    .authority(Some(&ctx.accounts.update_authority.to_account_info()))
    .system_program(&ctx.accounts.system_program.to_account_info())
    .plugin(Plugin::Attributes(Attributes {
        attribute_list: vec![Attribute {
            key: "staked_count".to_string(),
            value: ctx.accounts.config.staked_count.to_string(),
        }],
    }))
    .invoke_signed(&[signer_seeds])?;


    let amount=(staked_time as u64) 
    .checked_mul(ctx.accounts.config.rewards_bps as u64) 
    .ok_or( ErrorCode :: InvalidRewardsBps)?
    .checked_mul(10u64.pow( ctx.accounts.rewards_mint.decimals as u32))
    .ok_or(ErrorCode:: InvalidRewardsBps)? 
    .checked_div(10000u64)
    .ok_or(ErrorCode :: InvalidRewardsBps) ?;

    let config_seeds  = &[
        b"config",
        collection_key.as_ref(),
        &[ctx.accounts.config.bump],
    ];

    let config_signer_seeds: &[&[&[u8]]; 1] = &[&config_seeds[ .. ]];

    if amount > 0 {
        mint_to_checked(
            CpiContext :: new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintToChecked {
                    mint: ctx.accounts.rewards_mint.to_account_info(),
                    to: ctx.accounts.user_rewards_ata. to_account_info(),
                    authority: ctx.accounts.config. to_account_info(),
                },
                config_signer_seeds,
            ),
            amount,
            ctx.accounts.rewards_mint.decimals,
        )?;
    }

    Ok(())

}
