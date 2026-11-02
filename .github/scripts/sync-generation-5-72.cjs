// Capture reviewed public contracts without submitting work or changing accounts.
// Usage: node .github/scripts/sync-generation-5-72.cjs ../sogni-client
const fs = require('node:fs');
const path = require('node:path');
const { EventEmitter } = require('node:events');
const { execFileSync } = require('node:child_process');
const upstream = path.resolve(process.argv[2] || '../sogni-client');
const load = relative => require(path.join(upstream, relative));
const commit = execFileSync('git', ['-C', upstream, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
const version = load('package.json').version;
const protocolVersion = load('node_modules/@sogni-ai/sogni-protocol/package.json').version;
if (commit !== '0e5e31075fdeea276a4241c924bd3423c60804ce' || version !== '5.72.0'
  || load('dist/version.js').LIB_VERSION !== version || protocolVersion !== '1.0.0-alpha.55') {
  throw new Error('Build the reviewed TypeScript 5.72.0 revision first.');
}
const create = load('dist/Projects/createJobRequestMessage.js').default;
const utils = load('dist/Projects/utils/index.js');
const ProjectsApi = load('dist/Projects/index.js').default;
const cases = [], invalid = [];
const models = [
  ['minimax-h3-ref2va-fp8_r2v', 20],
  ['minimax-h3-ref2va-fp8_r2v_balanced', 8],
  ['minimax-h3-ref2va-fp8_r2v_turbo', 4],
  ['minimax-h3-fastvideo-omni-int8_r2v_turbo', 8],
];
function record(name, modelId, steps, extra, reject = false) {
  // true represents the presence of an already uploaded slot in wire tests.
  const params = { type:'video', modelId, steps, guidance:1, scheduler:'simple',
    positivePrompt:'A character walks.', numberOfMedia:1, frames:124,
    width:1344, height:768, referenceVideo:true, ...extra };
  const options = { type:'video', steps:{min:steps,max:steps,step:1,default:steps},
    guidance:{min:1,max:1,step:1,default:1}, fps:{allowed:[24],default:24},
    sampler:{allowed:[],default:null}, scheduler:{allowed:['simple'],default:'simple'} };
  try {
    const wire = create('UPSTREAM-5-72', params, options);
    if (reject) throw new Error('Expected rejection: ' + name);
    cases.push({name,params,options,wire});
  } catch (error) {
    if (!reject || error.message.startsWith('Expected rejection:')) throw error;
    invalid.push({name,params,options,message:error.message});
  }
}
for (const [model,steps] of models) {
  for (const mode of [undefined,'none','vpack']) record(model+'-'+mode,model,steps,{referenceVideoCompression:mode});
  record(model+'-array',model,steps,{referenceVideo:undefined,referenceVideos:[true],referenceVideoCompression:'vpack'});
  if (utils.isMinimaxH3Model(model+'_2stage')) {
    record(model+'-2stage-none',model+'_2stage',steps,{referenceVideoCompression:'none'});
    record(model+'-2stage-vpack',model+'_2stage',steps,{referenceVideoCompression:'vpack'},true);
  }
  record(model+'-image-only',model,steps,{referenceVideo:undefined,referenceImage:true,referenceVideoCompression:'vpack'},true);
}
for (const mode of ['automatic',null,true,0]) record('bad-mode-'+mode,models[1][0],8,{referenceVideoCompression:mode},true);
record('t2v-vpack','minimax-h3-fl2va-fp8_t2v',20,{referenceVideoCompression:'vpack'},true);
const seedream = 'seedream-5-0-pro-uncensored';
const imageOptions = {type:'image',sampler:{allowed:[],default:null},scheduler:{allowed:[],default:null}};
for (const outputFormat of [undefined,'png','jpg','webp']) {
  for (const references of [false,true]) {
    const params = {type:'image',modelId:seedream,positivePrompt:'A portrait',numberOfMedia:1,
      outputFormat, ...(references ? {contextImages:[true]} : {})};
    cases.push({name:`seedream-${outputFormat}-${references}`,params,options:imageOptions,wire:create('UPSTREAM-5-72',params,imageOptions)});
  }
}
async function main() {
  const client = Object.assign(new EventEmitter(), {appId:'fixture',
    logger:{error(){},warn(){},info(){},debug(){}}, socket:new EventEmitter(), rest:{}});
  client.socket.get = async () => { throw Object.assign(new Error('Not Found'), {status:404}); };
  const api = new ProjectsApi({client,eip712:{}});
  api._listActiveProjectIds = async () => null;
  const failures = [];
  const inputs = [undefined,null,{},[],17,'failure',
    {code:'PROVIDER_CODE',category:'content_policy',message:'Public explanation',extra:'not public'},
    {code:'future-code'}, {category:'future_category'}, {message:' '},
    {code:17,category:false,message:'usable',extra:{private:true}},
    {code:'',category:'',message:''}];
  try {
    for (const [index,value] of inputs.entries()) {
      const snapshot = {id:'P',status:'failed',finished:true,statusOnly:true,workerJobs:[],completedWorkerJobs:[],
        ...(value === undefined ? {} : {failure:value})};
      api.getStatus = async () => snapshot;
      failures.push({name:'failure-'+index,snapshot,expected:await api.getResult('P')});
    }
  } finally { if (typeof api._stopTimers === 'function') api._stopTimers(); }
  const output = {version,commit,protocolVersion,cases,invalid,failures};
  fs.writeFileSync(path.resolve(__dirname,'../../src/projects/wire/fixtures/upstream-5-72.json'),JSON.stringify(output,null,2)+'\n');
  console.log(`Captured ${cases.length} wire cases, ${invalid.length} refusals and ${failures.length} public failure projections.`);
}
main().catch(error => {console.error(error);process.exitCode=1;});
