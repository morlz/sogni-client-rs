// Capture the public TypeScript 5.69.1 wire contract without network submission.
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const upstream = path.resolve(process.argv[2] || '../sogni-client');
const load = relative => require(path.join(upstream, relative));
const commit = execFileSync('git',['-C',upstream,'rev-parse','HEAD'],{encoding:'utf8'}).trim();
const version = load('package.json').version;
const protocolVersion = load('node_modules/@sogni-ai/sogni-protocol/package.json').version;
if(commit !== '5970a1bd40b88f678719188969c8633bc313b665' || version !== '5.69.1' || load('dist/version.js').LIB_VERSION !== version || protocolVersion !== '1.0.0-alpha.55') {
  throw new Error('Build the reviewed TypeScript 5.69.1 revision and protocol alpha.55 first.');
}
const create = load('dist/Projects/createJobRequestMessage.js').default;
const utils = load('dist/Projects/utils/index.js');
const ChatTools = load('dist/Chat/ChatTools.js').default;
const seedream = 'seedream-5-0-pro-uncensored';
const extended = [
  ['minimax-h3-fl2va-fp8_t2v',20],['minimax-h3-fl2va-fp8_t2v_balanced',8],
  ['minimax-h3-fastvideo-int8_t2v_turbo',4],['minimax-h3-fastvideo-int8_t2v_turbo_2stage',4],
];
const references = ['minimax-h3-fastvideo-omni-int8_r2v_turbo','minimax-h3-fastvideo-omni-int8_r2v_turbo_2stage'];
const optionsFor = params => ({
  type:params.type,
  sampler:{allowed:[],default:null},
  scheduler:{allowed:['simple'],default:'simple'},
  ...(params.type==='video'?{steps:{min:params.steps,max:params.steps,step:1,default:params.steps},guidance:{min:1,max:1,step:1,default:1},fps:{allowed:[24],default:24}}:{}),
});
const video = (modelId,steps,values={}) => ({type:'video',modelId,positivePrompt:'Continue the scene',numberOfMedia:1,width:1344,height:768,frames:141,steps,guidance:1,...values});
const image = values => ({type:'image',modelId:seedream,positivePrompt:'A portrait in window light',numberOfMedia:1,...values});
const cases=[], invalid=[];
function record(name,params,reject=false) {
  const options=optionsFor(params);
  try {
    const wire=create('UPSTREAM-5-69',params,options);
    if(reject) throw new Error(`Expected rejection: ${name}`);
    cases.push({name,params,options,wire});
  } catch(error) {
    if(!reject || error.message.startsWith('Expected rejection:')) throw error;
    invalid.push({name,params,options,message:error.message});
  }
}
for(const [model,steps] of extended) {
  record(`${model}-plain`,video(model,steps));
  record(`${model}-source-default`,video(model,steps,{referenceVideo:true}));
  for(const extendOutput of ['extended','continuation']) record(`${model}-${extendOutput}`,video(model,steps,{referenceVideo:true,extendOutput}));
  record(`${model}-bad-output`,video(model,steps,{referenceVideo:true,extendOutput:'both'}),true);
  record(`${model}-output-without-source`,video(model,steps,{extendOutput:'continuation'}),true);
}
record('lightx2v-cannot-extend',video('minimax-h3-fl2va-fp8_t2v_turbo',4,{referenceVideo:true}),true);
record('extend-external-url',video(extended[0][0],20,{referenceVideoUrls:['https://example.com/source.mp4']}),true);
for(const model of references) {
  record(`${model}-references`,video(model,8,{contextImages:[true],referenceVideo:true,referenceAudio:true,referenceVideoDurations:[5]}));
  record(`${model}-keyframes-personal-loras`,video(model,8,{referenceImage:true,contextImages:[true],keyframes:[{image:true,frameIndex:48},{image:true,frameIndex:96}],loras:['public-synthetic-adapter'],loraStrengths:[0.7]}));
  for(const steps of [4,20]) record(`${model}-wrong-steps-${steps}`,video(model,steps,{referenceImage:true}),true);
  record(`${model}-missing-visual`,video(model,8,{referenceAudio:true}),true);
}
for(const [width,height] of [[1024,1024],[1280,720],[1536,1536],[2048,2048],[2304,1728],[2560,1440],[3024,1296],[3840,240],[8192,512]]) {
  record(`seedream-${width}x${height}`,image({width,height}));
}
record('seedream-defaults',image({negativePrompt:'blurry'}));
record('seedream-preset',image({sizePreset:'square_hd',width:512,height:512}));
for(const count of [0,1,10]) record(`seedream-references-${count}`,image({contextImages:Array(count).fill(true)}));
for(const [width,height] of [[1024,768],[2048,2049],[4096,240],[239,4096],[8193,512]]) record(`seedream-invalid-${width}x${height}`,image({width,height}),true);
record('seedream-eleven-references',image({contextImages:Array(11).fill(true)}),true);
record('seedream-empty-reference',image({contextImages:[true,null]}),true);

async function main() {
  const png='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=';
  const models=[{id:seedream,media:'image',workerCount:50},{id:'gpt-image-2',media:'image',workerCount:9},{id:'qwen_image_edit_2511_fp8_lightning',media:'image',workerCount:1}];
  const tools=new ChatTools({waitForModels:async()=>models});
  tools.executeProject=async(_call,_type,_model,params)=>params;
  const toolCases=[];
  async function normalized(value) {
    if(value instanceof Blob) return true;
    if(Array.isArray(value)) return Promise.all(value.map(normalized));
    if(value && typeof value==='object') return Object.fromEntries(await Promise.all(Object.entries(value).map(async([key,v])=>[key,await normalized(v)])));
    return value;
  }
  for(const [tool,method,args] of [
    ['generate_image','executeImageGeneration',{prompt:'a portrait'}],
    ['edit_image','executeImageEdit',{prompt:'an edit',source_image_url:png}],
    ['generate_image','executeImageGeneration',{model:'Seedream 5.0 Pro Uncensored',prompt:'a portrait',negative_prompt:'blurry',steps:30,seed:7,width:2048,height:2048}],
    ['edit_image','executeImageEdit',{model:seedream,prompt:'an edit',source_image_url:png,reference_image_urls:[png],negative_prompt:'blurry',seed:7}],
  ]) {
    const result=await tools[method]({id:'fixture',function:{name:tool}},args,{});
    toolCases.push({tool,args,expected:await normalized(result)});
  }
  const predicates=[...extended.map(([id])=>id),...references,'minimax-h3-fl2va-fp8_t2v_turbo','minimax-h3-fastvideo-int8_i2v_turbo'].map(id=>({id,extend:utils.isMinimaxH3ExtendModel(id),fasth3Reference:utils.isMinimaxH3FastH3ReferenceModel(id),turbo:utils.isMinimaxH3TurboModel(id),keyframes:utils.isMinimaxH3KeyframeModel(id),assets:utils.getVideoAssetRequirements(id)}));
  const target=path.resolve(__dirname,'../../src/projects/wire/fixtures/upstream-5-69.json');
  fs.writeFileSync(target,JSON.stringify({version,commit,protocolVersion,cases,invalid,models,toolCases,predicates},null,2)+'\n');
  console.log(`Captured ${cases.length} wires, ${invalid.length} refusals and ${toolCases.length} tool requests.`);
}
main().catch(error=>{console.error(error);process.exitCode=1;});
