#![allow(unused)]
use bitcoin::hex::DisplayHex;
use bitcoincore_rpc::bitcoin::key::rand::seq;
use bitcoincore_rpc::bitcoin::key::Secp256k1;
use bitcoincore_rpc::bitcoin::{
    hex, Address, Amount, BlockHash, Network, PublicKey, ScriptBuf, Transaction, Txid,
};
use bitcoincore_rpc::json::LoadWalletResult;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use serde::Deserialize;
use serde_json::json;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::Write;
use std::ops::Add;
use std::str::FromStr;

// Node access params
const RPC_URL: &str = "http://127.0.0.1:18443"; // Default regtest RPC port
const RPC_USER: &str = "alice";
const RPC_PASS: &str = "password";

// You can use calls not provided in RPC lib API using the generic `call` function.
// An example of using the `send` RPC call, which doesn't have exposed API.
// You can also use serde_json `Deserialize` derivation to capture the returned json result.
fn send(
    rpc: &Client,
    addr: &str,
    amt: Amount,
    txid: &str,
    vout: u32,
) -> bitcoincore_rpc::Result<String> {
    let args = [
        json!([{addr : amt.to_float_in(bitcoincore_rpc::bitcoin::Denomination::Bitcoin) }]), // recipient address
        json!(null),                                     // conf target
        json!(null),                                     // estimate mode
        json!(null),                                     // fee rate in sats/vb
        json!({"inputs": [{"txid":txid, "vout":vout}]}), // Empty option object
    ];

    #[derive(Deserialize)]
    struct SendResult {
        complete: bool,
        txid: String,
    }
    let send_result = rpc.call::<SendResult>("send", &args)?;
    assert!(send_result.complete);
    Ok(send_result.txid)
}

// e1ec30: A little helper to convert a script to an address
fn script_to_addr(script: &ScriptBuf) -> Address {
    Address::from_script(script, Network::Regtest).unwrap()
}

// e1ec30: Check if address in script belongs to wallet
fn is_mine(rpc: &Client, script: &ScriptBuf) -> bool {
    let addr = script_to_addr(script);
    rpc.get_address_info(&addr).unwrap().is_mine.unwrap()
}

// e1ec30: Create a new rpc client each time I need to do something at a specific url
fn get_client_at_url(url: &str) -> bitcoincore_rpc::Result<Client> {
    let new_url = format!("{RPC_URL}{url}");
    let client = Client::new(
        &new_url,
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;
    Ok(client)
}

// e1ec30: A little helper to first try loading the wallet before creating it
fn load_or_create_wallet(name: &str, rpc: &Client) -> bitcoincore_rpc::Result<LoadWalletResult> {
    let wallet = rpc.load_wallet(name);

    match wallet {
        Ok(wallet) => Ok(wallet),
        Err(bitcoincore_rpc::Error::JsonRpc(e))
            if e.to_string().contains("Path does not exist") =>
        {
            let wallet = rpc.create_wallet(name, None, None, None, None)?;
            Ok(wallet)
        }
        Err(e) => Err(e),
    }
}

fn main() -> bitcoincore_rpc::Result<()> {
    // Connect to Bitcoin Core RPC
    let rpc = Client::new(
        RPC_URL,
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // Get blockchain info
    let blockchain_info = rpc.get_blockchain_info()?;
    println!("Blockchain Info: {blockchain_info:?}");

    // Create/Load the wallets, named 'Miner' and 'Trader'. Have logic to optionally create/load them if they do not exist or not loaded already.
    load_or_create_wallet("Trader", &rpc)?;
    load_or_create_wallet("Miner", &rpc)?;

    // println!("Miner wallet created: {miner_wallet:?}");
    // println!("Trader wallet created: {trader_wallet:?}");

    // Generate spendable balances in the Miner wallet. How many blocks needs to be mined?
    let miner_wallet_rpc = get_client_at_url("/wallet/Miner")?;
    let miner_address = miner_wallet_rpc
        .get_new_address(None, None)?
        .assume_checked();
    let block = miner_wallet_rpc.generate_to_address(101, &miner_address)?;

    // e1ec30: Get a single utxo that can be used in the transaction, since the tests require it
    let unspent = miner_wallet_rpc.list_unspent(None, None, None, None, None)?;
    let viable = unspent.iter().find(|u| u.amount.to_btc() > 20.0).unwrap();

    // Load Trader wallet and generate a new address
    let trader_wallet_rpc = get_client_at_url("/wallet/Trader")?;
    let trader_address = trader_wallet_rpc
        .get_new_address(None, None)?
        .assume_checked();
    // println!("trader_address: {trader_address}");

    // Send 20 BTC from Miner to Trader
    let txhash = send(
        &miner_wallet_rpc,
        &trader_address.to_string(),
        Amount::from_int_btc(20),
        &viable.txid.to_string(),
        viable.vout,
    )?;
    // println!("Transaction Hash: {txhash}");

    // Check transaction in mempool
    let txid_transfer = Txid::from_str(&txhash).unwrap();
    let tx_res = miner_wallet_rpc.get_transaction(&txid_transfer, None)?;
    let fee = tx_res.fee.unwrap();

    // Mine 1 block to confirm the transaction
    let mined = miner_wallet_rpc.generate_to_address(1, &miner_address)?;

    // Extract all required transaction details
    let block = miner_wallet_rpc.get_block(&mined[0])?;

    // e1ec30: Find my transaction in the block
    let confirmed_tx = block
        .txdata
        .iter()
        .find(|tx| tx.txid() == txid_transfer)
        .unwrap();

    // e1ec30: Also get the transaction containing the input I used
    let input_tx = miner_wallet_rpc.get_raw_transaction(&viable.txid, None)?;

    // e1ec30: Extract Miner's input address and amount
    let output_spent = input_tx.output.get(viable.vout as usize).unwrap();
    let miner_in_addr = script_to_addr(&output_spent.script_pubkey);
    let miner_in_amount = output_spent.value.to_btc();

    // e1ec30: Extract Trader's Output address and amount
    let trader_out = confirmed_tx
        .output
        .iter()
        .find(|o| is_mine(&trader_wallet_rpc, &o.script_pubkey))
        .unwrap();
    let trader_out_addr = script_to_addr(&trader_out.script_pubkey);
    let trader_amount = trader_out.value.to_btc();

    // e1ec30: Extract Miner's Change address and amount
    let miner_change = confirmed_tx
        .output
        .iter()
        .find(|o| is_mine(&miner_wallet_rpc, &o.script_pubkey))
        .unwrap();
    let miner_change_addr = script_to_addr(&miner_change.script_pubkey);
    let miner_amount = miner_change.value.to_btc();

    // Write the data to ../out.txt in the specified format given in readme.md
    let mut f = File::create("../out.txt").unwrap();
    writeln!(f, "{}", confirmed_tx.txid());
    writeln!(f, "{miner_in_addr}");
    writeln!(f, "{miner_in_amount}");
    writeln!(f, "{trader_out_addr}");
    writeln!(f, "{trader_amount}");
    writeln!(f, "{miner_change_addr}");
    writeln!(f, "{miner_amount}");
    writeln!(f, "{fee}");
    writeln!(f, "{}", block.bip34_block_height().unwrap());
    writeln!(f, "{}", block.block_hash());

    // e1ec30: Forgot to enable GitHub Actions

    Ok(())
}
