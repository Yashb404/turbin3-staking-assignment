import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { AnchorCoreStaking } from "../target/types/anchor_core_staking";
import { SystemProgram } from "@solana/web3.js";
import { MPL_CORE_PROGRAM_ID, deserializeAssetV1, deserializeCollectionV1 } from "@metaplex-foundation/mpl-core";
import { createAmount, publicKey } from "@metaplex-foundation/umi";
import { ASSOCIATED_TOKEN_PROGRAM_ID, getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { expect } from "chai";

const MILLISECONDS_PER_DAY = 86400000;
const REWARDS_BPS = 10_000; 
const FREEZE_PERIOD_IN_DAYS = 7;
const TIME_TRAVEL_IN_DAYS = 8;

describe("anchor-core-staking", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.anchorCoreStaking as Program<AnchorCoreStaking>;
  const stakingProgram = program as Program<any>;

  const collectionKeypair = anchor.web3.Keypair.generate();

  const updateAuthority = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("update_authority"), collectionKeypair.publicKey.toBuffer()],
    program.programId
  )[0];

  const nftKeypair = anchor.web3.Keypair.generate();

  const config = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("config"), collectionKeypair.publicKey.toBuffer()],
    program.programId
  )[0];

  const rewardsMint = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("rewards_mint"), config.toBuffer()],
    program.programId
  )[0];

async function advanceTime(params: { absoluteEpoch ?: number; absoluteSlot ?: number; absoluteTimestamp ?: number }): Promise<void> {
    const rpcResponse = await fetch(provider.connection.rpcEndpoint, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "surfnet_timeTravel",
        params: [params],
    }),
    });

    const result = await rpcResponse.json() as { error ?: any; result ?: any };
    if (result.error) {
        throw new Error(`Time travel failed: ${JSON.stringify(result.error)}`);
    }

    await new Promise((resolve) => setTimeout(resolve, 1000));
}

  async function fetchAsset(address: anchor.web3.PublicKey) {
    const accountInfo = await provider.connection.getAccountInfo(address);
    if (!accountInfo) {
      throw new Error(`Missing asset account ${address.toBase58()}`);
    }

    return deserializeAssetV1({
      publicKey: publicKey(address.toBase58()),
      data: new Uint8Array(accountInfo.data),
      executable: accountInfo.executable,
      owner: publicKey(accountInfo.owner.toBase58()),
      lamports: createAmount(BigInt(accountInfo.lamports), "SOL", 9),
      rentEpoch: BigInt(accountInfo.rentEpoch),
    });
  }

  async function fetchCollection(address: anchor.web3.PublicKey) {
    const accountInfo = await provider.connection.getAccountInfo(address);
    if (!accountInfo) {
      throw new Error(`Missing collection account ${address.toBase58()}`);
    }

    return deserializeCollectionV1({
      publicKey: publicKey(address.toBase58()),
      data: new Uint8Array(accountInfo.data),
      executable: accountInfo.executable,
      owner: publicKey(accountInfo.owner.toBase58()),
      lamports: createAmount(BigInt(accountInfo.lamports), "SOL", 9),
      rentEpoch: BigInt(accountInfo.rentEpoch),
    });
  }

  function getAttributeValue(
    attributes: { attributeList: Array<{ key: string; value: string }> } | undefined,
    key: string
  ) {
    return attributes?.attributeList.find((attribute) => attribute.key === key)?.value;
  }

  it("Create a collection", async () => {
    const collectionName = "Test Collection";
    const collectionUri = "https://example.com/collection";
    const tx = await program.methods.createCollection(collectionName, collectionUri)
    .accountsPartial({
      payer: provider.wallet.publicKey,
      collection: collectionKeypair.publicKey,
      updateAuthority,
      systemProgram: SystemProgram.programId,
      mplCoreProgram: MPL_CORE_PROGRAM_ID,
    })
    .signers([collectionKeypair])
    .rpc();
    console.log("\nYour transaction signature", tx);
    console.log("Collection address", collectionKeypair.publicKey.toBase58());
  });

  it("Mint an NFT", async () => {
    const nftName = "Test NFT";
    const nftUri = "https://example.com/nft";
    const tx = await program.methods.mintAsset(nftName, nftUri)
    .accountsPartial({
      user: provider.wallet.publicKey,
      asset: nftKeypair.publicKey,
      collection: collectionKeypair.publicKey,
      updateAuthority,
      systemProgram: SystemProgram.programId,
      mplCoreProgram: MPL_CORE_PROGRAM_ID,
    })
    .signers([nftKeypair])
    .rpc();
    console.log("\nYour transaction signature", tx);
    console.log("NFT address", nftKeypair.publicKey.toBase58());
  });

  it("Initialize stake config", async () => {
    const tx = await program.methods.initialize(REWARDS_BPS, FREEZE_PERIOD_IN_DAYS)
    .accountsPartial({
      admin: provider.wallet.publicKey,
      collection: collectionKeypair.publicKey,
      updateAuthority,
      config,
      rewardsMint,
      systemProgram: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .rpc();
    console.log("\nYour transaction signature", tx);
    console.log("Config address", config.toBase58());
    console.log("Points per staked NFT per day", REWARDS_BPS);
    console.log("Freeze period in days", FREEZE_PERIOD_IN_DAYS);
    console.log("Rewards mint address", rewardsMint.toBase58());
  });

  it("Stake an NFT", async () => {
    const tx = await program.methods.stake()
    .accountsPartial({
      owner: provider.wallet.publicKey,
      updateAuthority,
      config,
      asset: nftKeypair.publicKey,
      collection: collectionKeypair.publicKey,
      systemProgram: SystemProgram.programId,
      mplCoreProgram: MPL_CORE_PROGRAM_ID,
    })
    .rpc();
    console.log("\nYour transaction signature", tx);

    const asset = await fetchAsset(nftKeypair.publicKey);
    const collection = await fetchCollection(collectionKeypair.publicKey);

    expect(getAttributeValue(asset.attributes, "staked")).to.eq("true");
    expect(getAttributeValue(asset.attributes, "staked_at")).to.not.eq(undefined);
    expect(getAttributeValue(asset.attributes, "rewards_updated_at")).to.not.eq(undefined);
    expect(getAttributeValue(collection.attributes, "staked_count")).to.eq("1");
  });

  it("Try to unstake an NFT before the freeze period ends", async () => {
    //Get the user rewards ATA acount

    const userRewardsAta = getAssociatedTokenAddressSync(rewardsMint, provider.wallet.publicKey, false, TOKEN_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID);
    try {
        const tx = await program.methods.unstake()
        .accountsPartial({
            owner: provider.wallet.publicKey,
            updateAuthority,
            config,
            rewardsMint,
            userRewardsAta,
            asset: nftKeypair.publicKey,
            collection: collectionKeypair.publicKey,
            mplCoreProgram: MPL_CORE_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        })
        .rpc();
        throw new Error(`Unstake should have failed before freeze period elapsed, but succeeded with tx: ${tx}`);
        } catch (err) {
        if (err instanceof anchor.AnchorError && err.error.errorCode.code === "FreezePeriodNotElapsed") {
            console. log("\nUnstake failed as expected:", err.error.errorMessage);
        } else {
            throw err;
        }
    }
  });

  it("Time travel to the future (Surfpool only)", async function() {
    try {
      const currentTimestamp = Date.now();
      await advanceTime({ absoluteTimestamp: currentTimestamp + TIME_TRAVEL_IN_DAYS * MILLISECONDS_PER_DAY });
      console.log("\nTime traveled in days", TIME_TRAVEL_IN_DAYS);
    } catch (err) {
      console.log("\nSkipping time travel: Surfpool not available");
      this.skip();
    }
  });

  it("Claim rewards and unstake an NFT", async () => {
    const userRewardsAta = getAssociatedTokenAddressSync(rewardsMint, provider.wallet.publicKey, false, TOKEN_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID);
    const beforeClaimBalance = (await provider.connection.getTokenAccountBalance(userRewardsAta)).value.uiAmount;
    const tx = await stakingProgram.methods.claim()
    .accountsPartial({
      owner: provider.wallet.publicKey,
      updateAuthority,
      config,
      rewardsMint,
      userRewardsAta,
      asset: nftKeypair.publicKey,
      collection: collectionKeypair.publicKey,
      mplCoreProgram: MPL_CORE_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    })
    .rpc();
    console.log("\nClaim transaction signature", tx);

    const afterClaimBalance = (await provider.connection.getTokenAccountBalance(userRewardsAta)).value.uiAmount;
    expect(afterClaimBalance).to.be.greaterThan(beforeClaimBalance ?? 0);

    const claimedAsset = await fetchAsset(nftKeypair.publicKey);
    const claimUpdatedAt = getAttributeValue(claimedAsset.attributes, "rewards_updated_at");
    expect(getAttributeValue(claimedAsset.attributes, "staked")).to.eq("true");
    expect(claimUpdatedAt).to.not.eq(undefined);

    const unstakeTx = await program.methods.unstake()
    .accountsPartial({
      owner: provider.wallet.publicKey,
      updateAuthority,
      config,
      rewardsMint,
      userRewardsAta,
      asset: nftKeypair.publicKey,
      collection: collectionKeypair.publicKey,
      mplCoreProgram: MPL_CORE_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    })
    .rpc();
    console.log("\nYour transaction signature", unstakeTx);

    const finalBalance = (await provider.connection.getTokenAccountBalance(userRewardsAta)).value.uiAmount;
    expect(finalBalance).to.eq(afterClaimBalance);

    const asset = await fetchAsset(nftKeypair.publicKey);
    const collection = await fetchCollection(collectionKeypair.publicKey);

    expect(getAttributeValue(asset.attributes, "staked")).to.eq("false");
    expect(getAttributeValue(asset.attributes, "staked_at")).to.eq("0");
    expect(getAttributeValue(asset.attributes, "rewards_updated_at")).to.eq("0");
    expect(getAttributeValue(collection.attributes, "staked_count")).to.eq("0");

    console.log("User rewards balance", finalBalance);
  });


});
