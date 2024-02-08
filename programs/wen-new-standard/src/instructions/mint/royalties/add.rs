use anchor_lang::{prelude::*, solana_program::entrypoint::ProgramResult};
use spl_tlv_account_resolution::{account::ExtraAccountMeta, state::ExtraAccountMetaList};

use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{
        spl_token_metadata_interface::state::Field, token_metadata_update_field,
        transfer_hook_update, Mint, Token2022, TokenMetadataUpdateField,
        TokenMetadataUpdateFieldArgs, TransferHookUpdate,
    },
};
use spl_transfer_hook_interface::instruction::ExecuteInstruction;

fn get_meta_list(approve_account: Pubkey) -> Vec<ExtraAccountMeta> {
    vec![
        // mint
        ExtraAccountMeta {
            discriminator: 0,
            address_config: approve_account.to_bytes(),
            is_signer: false.into(),
            is_writable: true.into(),
        },
    ]
}

fn get_meta_list_size() -> usize {
    ExtraAccountMetaList::size_of(1).unwrap()
}

use crate::{
    get_approve_account_pda, update_mint_lamports_to_minimum_balance, MetadataErrors,
    META_LIST_ACCOUNT_SEED, ROYALTY_BASIS_POINTS_FIELD,
};

#[derive(AnchorDeserialize, AnchorSerialize)]
pub struct CreatorWithShare {
    pub address: String,
    pub share: u8,
}

#[derive(AnchorDeserialize, AnchorSerialize)]
pub struct AddRoyaltiesArgs {
    pub royalty_basis_points: u16,
    pub creators: Vec<CreatorWithShare>,
}

#[derive(Accounts)]
#[instruction(args: AddRoyaltiesArgs)]
pub struct AddRoyalties<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        mint::token_program = token_program,
    )]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    /// CHECK: This account's data is a buffer of TLV data
    #[account(
        init_if_needed,
        space = get_meta_list_size(),
        seeds = [META_LIST_ACCOUNT_SEED, mint.key().as_ref()],
        bump,
        payer = payer,
    )]
    pub extra_metas_account: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Program<'info, Token2022>,
}

impl<'info> AddRoyalties<'info> {
    fn update_token_metadata_field(&self, field: Field, value: String) -> ProgramResult {
        let cpi_accounts = TokenMetadataUpdateField {
            token_program_id: self.token_program.to_account_info(),
            metadata: self.mint.to_account_info(),
            update_authority: self.authority.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(self.token_program.to_account_info(), cpi_accounts);
        token_metadata_update_field(cpi_ctx, TokenMetadataUpdateFieldArgs { field, value })?;
        Ok(())
    }

    fn update_transfer_hook_program_id(&self) -> Result<()> {
        let cpi_accounts = TransferHookUpdate {
            token_program_id: self.token_program.to_account_info(),
            mint: self.mint.to_account_info(),
            authority: self.authority.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(self.token_program.to_account_info(), cpi_accounts);
        transfer_hook_update(cpi_ctx, Some(crate::id()))?;
        Ok(())
    }
}

pub fn handler(ctx: Context<AddRoyalties>, args: AddRoyaltiesArgs) -> Result<()> {
    // add royalty basis points to metadata
    ctx.accounts.update_token_metadata_field(
        Field::Key(ROYALTY_BASIS_POINTS_FIELD.to_owned()),
        args.royalty_basis_points.to_string(),
    )?;

    let mut total_share: u8 = 0;
    // add creators and their respective shares to metadata
    for creator in args.creators {
        let total_share_res = total_share.checked_add(creator.share);
        if total_share_res.is_some() {
            total_share = total_share_res.unwrap();
        } else {
            return Err(MetadataErrors::CreatorShareInvalid.into());
        }
        ctx.accounts
            .update_token_metadata_field(Field::Key(creator.address), creator.share.to_string())?;
    }

    if total_share != 100 {
        return Err(MetadataErrors::CreatorShareInvalid.into());
    }

    // initialize the extra metas account
    let extra_metas_account = &ctx.accounts.extra_metas_account;
    let approve_account = get_approve_account_pda(ctx.accounts.mint.to_account_info().key());
    let metas = get_meta_list(approve_account);
    let mut data = extra_metas_account.try_borrow_mut_data()?;
    ExtraAccountMetaList::init::<ExecuteInstruction>(&mut data, &metas)?;

    // add metadata program as the transfer hook program
    ctx.accounts.update_transfer_hook_program_id()?;

    update_mint_lamports_to_minimum_balance(
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.payer.to_account_info(),
        ctx.accounts.system_program.to_account_info(),
    )?;

    Ok(())
}
