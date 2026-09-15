// Rebuild the upstream SDK first: npm ci && npm run build.
// Usage: node scripts/update-generation-parity-fixtures.cjs ../sogni-client
// Fixtures come from the actual upstream serializer; no Rust implementation is
// involved in producing expected wires, errors, or mapped capability tiers.
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
if (!process.argv[2]) throw new Error('Pass the built upstream sogni-client checkout path.');
const root = path.resolve(process.argv[2]);
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd:root, encoding:'utf8' }).trim();
const version = require(path.join(root, 'package.json')).version;
const builtVersion = require(path.join(root, 'dist/version.js')).LIB_VERSION;
if (commit !== '452e78967a21ab80977c11f16517072d1836405a' || version !== '5.50.0') {
  throw new Error('Update the pinned upstream revision intentionally before regenerating fixtures.');
}
if (builtVersion !== version) throw new Error('Rebuild the upstream SDK before generating fixtures.');
const create = require(path.join(root, 'dist/Projects/createJobRequestMessage.js')).default;
const map = require(path.join(root, 'dist/Projects/types/ModelOptions.js'));
const utils = require(path.join(root, 'dist/Projects/utils/index.js'));
const destination = path.resolve(__dirname, '../src/projects/wire/fixtures');
const old = JSON.parse(fs.readFileSync(path.join(destination, 'utility-contract.json')));
for (const fixture of old.cases) fixture.wire = create('UPSTREAM-FIXTURE', fixture.params, fixture.options);
old.upstream = 'Sogni-AI/sogni-client@452e789 (5.50.0)';
fs.writeFileSync(path.join(destination, 'utility-contract.json'), JSON.stringify(old, null, 2) + '\n');
const defaults = type => ({type, sampler: {allowed:['euler'],default:'euler'}, scheduler:{allowed:['simple'],default:'simple'}});
const cases = [], invalid = [];
const params = (type, modelId, values={}) => ({type,modelId,positivePrompt:'',numberOfMedia:1,...values});
function add(name, input, options=defaults(input.type)) { cases.push({name, params:input, options, wire:create('UPSTREAM-FIXTURE', input, options)}); }
function reject(name, input, options=defaults(input.type)) {
  try { create('UPSTREAM-FIXTURE', input, options); throw new Error(`No rejection for ${name}`); }
  catch (error) { if (error.message.startsWith('No rejection')) throw error; invalid.push({name, params:input, options, message:error.message}); }
}
const sam = changes => params('image',utils.SAM3_IMAGE_SEGMENT_MODEL_ID,{startingImage:true,...changes});
for (const sam3Prompt of [
  {text:'dogs',multimask:false,applyMask:true,maxInstances:1,boxes:[{x0:0,y0:0,x1:.5,y1:.5,label:'negative'}]},
  {boxes:[{x0:0,y0:0,x1:1,y1:1}]},
  {points:[{x:.5,y:.5,label:'positive'}],multimask:false,maxInstances:16},
]) add(`sam-${cases.length}`, sam({sam3Prompt}));
for (const sam3Prompt of [{text:'dogs',multimask:true},{text:'dogs',applyMask:1},{text:'dogs',maxInstances:17},{text:'dogs',maxInstances:1.5},{boxes:[{x0:0,y0:0,x1:1,y1:1,label:'other'}]},{points:[{x:.5,y:.5,label:'positive'}],boxes:[{x0:0,y0:0,x1:1,y1:1,label:'negative'}]}]) reject(`sam-invalid-${invalid.length}`,sam({sam3Prompt}));
const biref = changes => params('image',utils.BIREFNET_BACKGROUND_REMOVAL_MODEL_ID,{startingImage:true,numberOfMedia:4,numberOfPreviews:6,outputFormat:'jpg',...changes});
add('biref-matte',biref()); add('biref-cutout',biref({applyMask:true}));
reject('biref-no-source',biref({startingImage:false})); reject('biref-invalid-mask',biref({applyMask:null}));
reject('biref-field-other-model',params('image','flux1-schnell-fp8',{applyMask:false}));
const pixal = (modelId,changes={}) => params('image',modelId,{startingImage:true,numberOfPreviews:4,...changes});
add('pixal-single-options',pixal(utils.PIXAL3D_IMAGE_TO_3D_MODEL_ID,{templateVariant:'i23d-birefnet',textureSize:1024,meshTargetFaces:5000,normalMapSize:512,ambientOcclusionSize:256,shapeResolution:1536}));
add('pixal-right-only',pixal(utils.PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID,{rightViewImage:true}));
add('pixal-all-views',pixal(utils.PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID,{leftViewImage:true,backViewImage:true,rightViewImage:true,meshTargetFaces:60000}));
for (const values of [{templateVariant:'i23d'},{leftViewImage:true},{contextImages:[true]},{meshTargetFaces:4999},{shapeResolution:1537},{textureSize:1024.5}]) reject(`pixal-invalid-${invalid.length}`,pixal(utils.PIXAL3D_IMAGE_TO_3D_MODEL_ID,values));
for (const values of [{startingImage:false},{rightViewImage:false},{templateVariant:'i23d-birefnet'},{contextImages:[true]}]) reject(`pixal-multi-invalid-${invalid.length}`,pixal(utils.PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID,values));
for (const mode of ['custom_voice','voice_design','voice_clone']) {
  const controls = mode==='custom_voice' ? {speaker:'ryan',instruct:'Speak quietly.'} : mode==='voice_design' ? {instruct:'A calm adult voice.'} : {referenceAudio:true,referenceText:'A short reference.'};
  add(`speech-${mode}`,params('audio',`qwen3_tts_1.7b_${mode}_bf16`,{positivePrompt:'Hello there.',language:'en',steps:1,...controls}),{type:'audio',steps:{min:1,max:1,step:1,default:1}});
}
add('speech-no-sampler',params('audio','qwen3_tts_1.7b_custom_voice_bf16',{sampler:'unused',scheduler:'unused'}),{type:'audio'});
for (const model of ['gpt-image-2','gpt-image-2.5-sunburst','gpt-image-2.5-flare']) {
  add(`gpt-${model}`,params('image',model,{sizePreset:'custom',width:3840,height:256,contextImages:Array(16).fill(true),gptImageQuality:model==='gpt-image-2'?'high':'max',gptImageBackground:model==='gpt-image-2'?'opaque':'transparent',gptImageOutputCompression:80,outputFormat:'webp',gptImageMask:true}));
  reject(`gpt-${model}-auto`,params('image',model,{gptImageQuality:'auto'}));
}
for (const values of [{gptImageMask:true},{gptImageMaskUrl:'',contextImages:[true]},{gptImageMask:true,gptImageMaskUrl:'https://example.com/mask.png',contextImages:[true]},{contextImages:Array(17).fill(true)},{contextImages:[false]},{gptImageBackground:'transparent',outputFormat:'jpg'},{gptImageOutputCompression:1.5,outputFormat:'webp'},{gptImageOutputCompression:80,outputFormat:'png'}]) reject(`gpt-invalid-${invalid.length}`,params('image','gpt-image-2.5-flare',values));
reject('gpt-2-max',params('image','gpt-image-2',{gptImageQuality:'max'}));
reject('gpt-mask-wrong-model',params('image','flux1-schnell-fp8',{gptImageMask:true}));
const flash = values => params('video',utils.FLASHVSR_VIDEO_UPSCALE_MODEL_ID,{referenceVideo:true,upscaleResolution:1440,...values});
add('flash-minimal',flash()); add('flash-exact-count',flash({frames:158,fps:24000/1001,duration:5,width:2520,height:1440}));
add('flash-long-source',flash({duration:120,fps:60,detailPreference:'sharper',processingSpeed:'faster',seed:-1}));
for (const values of [{fps:120},{frames:0},{frames:158.5},{duration:5},{positivePrompt:'repaint'},{negativePrompt:'blurry'},{teacacheThreshold:0},{generateAudio:false},{numberOfMedia:2},{detailPreference:'auto'},{processingSpeed:'auto'},{seed:'42'},{seed:-2},{seed:4294967296},{referenceImage:true},{upscaleResolution:2160}]) reject(`flash-invalid-${invalid.length}`,flash(values));
const h3 = (mode,suffix='',values={}) => params('video',`minimax-h3-fastvideo-int8_${mode}_turbo${suffix}`,{frames:243,width:672,height:384,steps:4,guidance:1,...(['i2v','flf2v','ia2v','flfa2v'].includes(mode)?{referenceImage:true}:{}),...(['flf2v','flfa2v'].includes(mode)?{referenceImageEnd:true}:{}),...(['ia2v','flfa2v','a2v'].includes(mode)?{referenceAudio:true}:{}),...values});
for (const mode of ['t2v','i2v','flf2v','ia2v','flfa2v','a2v']) for (const suffix of ['','_2stage']) add(`h3-${mode}${suffix}`,h3(mode,suffix));
add('h3-audio-offset',h3('a2v','_2stage',{audioStart:2.5,generateAudio:true,loras:[],loraStrengths:[]}));
for (const mode of ['ia2v','flfa2v','a2v']) {
  for (const values of [{referenceAudio:false},{generateAudio:false},{audioDuration:2},{audioStart:'1'},{audioStart:-1},{loras:['test']},{loraStrengths:[1]},{steps:8}]) reject(`h3-audio-invalid-${invalid.length}`,h3(mode,'_2stage',values));
}
for (const outputScale of [null,false,1,'2']) reject(`h3-output-scale-${invalid.length}`,h3('t2v','_2stage',{outputScale}));
for (const mode of ['t2v','i2v','flf2v']) reject(`h3-retired-720-${mode}`,h3(mode,'_2stage_720p'));
reject('h3-non-audio-offset',h3('t2v','',{audioStart:0}));
add('seedance-export',params('video','seedance-2-5',{positivePrompt:'A kite.',duration:5,outputFormat:'mov',returnLastFrame:true}));
for (const values of [{outputFormat:'webm'},{outputFormat:'mov'},{returnLastFrame:true},{returnLastFrame:null}]) reject(`export-invalid-${invalid.length}`,params('video','seedance-2-0',values));
add('receipt-application-chosen',params('image','gpt-image-2.5-flare',{appSource:'custom-application',worldGenerationReceipt:{stage:'target_still',sourceImageSha256:'A'.repeat(64),selectionHash:'B'.repeat(64)}}));
const tiers = [
  {name:'speech-custom',media:'audio',tier:{steps:{min:1,max:1,default:1},speaker:{allowed:['ryan'],default:'ryan'},instruct:{maxLength:4000},language:{allowed:['en'],default:'en'}}},
  {name:'speech-clone',media:'audio',tier:{steps:{min:1,max:1,default:1},referenceText:{maxLength:6000},requiresReferenceAudio:true}},
  {name:'speech-design',media:'audio',tier:{steps:{min:1,max:1,default:1},instruct:{maxLength:4000,required:true}}},
  {name:'flash',media:'video',tier:{type:'video',task:'video-upscale',outputResolutions:[1080,1440],preservesSourceTiming:true,requiresReferenceVideo:true,width:{min:2,max:2560,step:2,default:2520},height:{min:2,max:2560,step:2,default:1440},steps:{min:1,max:1,default:1},guidance:{min:1,max:1,default:1},fps:{min:1,max:60,default:24},comfySampler:{allowed:['euler'],default:'euler'},comfyScheduler:{allowed:['simple'],default:'simple'}}},
];
for (const tier of tiers) tier.options = (tier.media==='audio'?map.mapAudioTier:map.mapVideoTier)(tier.tier);
fs.writeFileSync(path.join(destination,'generation-contract.json'),JSON.stringify({upstream:old.upstream,cases,invalid,tiers},null,2)+'\n');
console.log({valid:cases.length,invalid:invalid.length,tiers:tiers.length});
