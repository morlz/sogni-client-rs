// Capture deterministic public, synthetic wallet vectors from the sibling SDK.
// Usage: node .github/scripts/sync-wallet-contract.cjs ../sogni-client
const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');
const { execFileSync } = require('node:child_process');
const upstream = path.resolve(process.argv[2] || '../sogni-client');
const requireUpstream = createRequire(path.join(upstream, 'package.json'));
const { TypedDataEncoder, keccak256, concat } = requireUpstream('ethers');
const Account = require(path.join(upstream, 'dist/Account/index.js')).default;
const commit = execFileSync('git', ['-C', upstream, 'rev-parse', 'HEAD'], { encoding:'utf8' }).trim();
const version = requireUpstream('./package.json').version;
if (commit !== '5970a1bd40b88f678719188969c8633bc313b665' || version !== '5.69.1') {
  throw new Error('Build the reviewed TypeScript 5.69.1 revision first.');
}
const wallets = [
  ['TestUser', 'correct horse battery staple'],
  ['testuser', 'correct horse battery staple'],
  ['Пользователь', 'public fixture 🔑'],
  [' User ', 'synthetic-only-password'],
].map(([username,password]) => ({ username, password, wallet: Account.prototype.getWallet.call({},username,password) }));
const domain = { name:'Sogni AI', version:'1', chainId:8453, verifyingContract:'0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC' };
const address = wallets[0].wallet.address.toLowerCase();
const authFields = [{name:'walletAddress',type:'address'},{name:'nonce',type:'string'}];
const cases = [];
function add(name, types, message, overrides = {}, primaryType) {
  primaryType ??= TypedDataEncoder.getPrimaryType(types);
  cases.push({name, wallet:0, data:{types, primaryType, domain:{...domain,...overrides}, message}});
}
add('authentication', {Authentication:authFields}, {walletAddress:address,nonce:'alpha-test-nonce'});
add('testnet-unicode', {Authentication:authFields}, {walletAddress:address,nonce:'校验 🔑'}, {name:'Sogni-testnet',chainId:84532});
add('permit-big-decimal', {Permit:[
  {name:'owner',type:'address'},{name:'spender',type:'address'},{name:'value',type:'uint256'},
  {name:'nonce',type:'uint256'},{name:'deadline',type:'uint256'}
]}, {owner:address,spender:'0x0000000000000000000000000000000000000001',
  value:'1234567890123456789012345678901234567890123456789012345678901234567890',nonce:'0x2105',deadline:'18446744073709551615'});
const primitiveFields = [
  {name:'flag',type:'bool'},{name:'small',type:'uint8'},{name:'signed',type:'int8'},
  {name:'large',type:'uint256'},{name:'minimum',type:'int256'},
  {name:'who',type:'address'},{name:'blob',type:'bytes'},{name:'tag',type:'bytes1'},
  {name:'digest',type:'bytes32'},{name:'text',type:'string'},
];
for (const flag of [false,true]) add(`primitives-${flag}`, {Primitive:primitiveFields}, {
  flag,small:255,signed:-128,large:((1n<<256n)-1n).toString(),minimum:(-(1n<<255n)).toString(),
  who:address,blob:flag?'0x':'0x012345',tag:'0xff',digest:'0x'+'ab'.repeat(32),text:'Exact text 山海\n',
});
add('nested-mail', {
  Person:[{name:'name',type:'string'},{name:'wallet',type:'address'}],
  Mail:[{name:'from',type:'Person'},{name:'to',type:'Person'},{name:'contents',type:'string'}],
}, {from:{name:'Cow',wallet:address},to:{name:'Bob',wallet:'0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB'},contents:'Hello, Bob!'});
const arrayTypes = {Batch:[{name:'numbers',type:'uint256[2]'},{name:'matrix',type:'int32[2][]'},{name:'texts',type:'string[]'},{name:'items',type:'Item[]'}],Item:[{name:'ok',type:'bool'},{name:'data',type:'bytes'}]};
add('arrays', arrayTypes, {numbers:[0,'0x2105'],matrix:[[-2147483648,2147483647],[0,-1]],texts:['','雪'],items:[{ok:true,data:'0x1234'},{ok:false,data:'0x'}]});
add('empty-arrays', arrayTypes, {numbers:[0,0],matrix:[],texts:[],items:[]});
add('domain-salt', {Authentication:authFields}, {walletAddress:address,nonce:'salt'}, {salt:'0x'+'42'.repeat(32)});
add('sparse-domain', {Authentication:authFields}, {walletAddress:address,nonce:'sparse'}, {name:undefined,version:undefined,chainId:undefined,verifyingContract:undefined});
cases.push({name:'domain-primary',wallet:0,data:{types:{EIP712Domain:[]},primaryType:'EIP712Domain',domain,message:{}}});
async function main() {
  for (const item of cases) {
    const {data}=item;
    item.hash=data.primaryType==='EIP712Domain'
      ? keccak256(concat(['0x1901',TypedDataEncoder.hashDomain(data.domain)]))
      : TypedDataEncoder.hash(data.domain,data.types,data.message);
    item.signature=wallets[item.wallet].wallet.signingKey.sign(item.hash).serialized;
    if(data.primaryType!=='EIP712Domain') {
      const signed=await wallets[item.wallet].wallet.signTypedData(data.domain,data.types,data.message);
      if(signed!==item.signature) throw new Error('Independent hash/sign path diverged');
    }
  }
  const output={version,commit,wallets:wallets.map(({username,password,wallet})=>({username,password,address:wallet.address.toLowerCase()})),cases};
  const target=path.resolve(__dirname,'../../src/account/fixtures/wallet-typescript.json');
  fs.mkdirSync(path.dirname(target),{recursive:true});
  fs.writeFileSync(target,JSON.stringify(output,null,2)+'\n');
  console.log(`Captured ${wallets.length} public wallet and ${cases.length} EIP-712 vectors without network submission.`);
}
main().catch(error=>{console.error(error);process.exitCode=1;});
