// Capture public 5.66.0 image contracts from the reviewed TypeScript SDK.
// Usage: node .github/scripts/sync-generation-5-66.cjs ../sogni-client
// The fixture distinguishes the low-level serializer from image-upload admission.
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const upstream = path.resolve(process.argv[2] || '../sogni-client');
const commit = execFileSync('git', ['-C', upstream, 'rev-parse', 'HEAD'], { encoding:'utf8' }).trim();
const version = require(path.join(upstream, 'package.json')).version;
const builtVersion = require(path.join(upstream, 'dist/version.js')).LIB_VERSION;
const protocolVersion = require(path.join(upstream, 'node_modules/@sogni-ai/sogni-protocol/package.json')).version;
if (commit !== '9805a8c146dea25065d080881e07da4c5c878a88' || version !== '5.66.0' || builtVersion !== version || protocolVersion !== '1.0.0-alpha.52') {
  throw new Error('Build the reviewed, pinned upstream 5.66.0 revision with protocol alpha.52 first.');
}
const create = require(path.join(upstream, 'dist/Projects/createJobRequestMessage.js')).default;
const Projects = require(path.join(upstream, 'dist/Projects/index.js')).default;
const ChatTools = require(path.join(upstream, 'dist/Chat/ChatTools.js')).default;
const { getMaxContextImages, isComfyModel } = require(path.join(upstream, 'dist/lib/validation.js'));
const moss = ['moss_image_sft_int8_convrot', 'moss_image_sft_dmd8_int8_convrot'];
const flux = 'flux3_image_klein_sd_int8_convrot';
const prompt = '{"caption":"A sign with exact text 山海","objects":["door"]}';
const optionsFor = modelId => ({
  type:'image',
  sampler:{ allowed:['euler'], default:'euler' },
  scheduler:{ allowed:[modelId === flux ? 'simple' : 'sgm_uniform'], default:modelId === flux ? 'simple' : 'sgm_uniform' }
});
const image = (modelId, values = {}) => ({ type:'image', modelId, positivePrompt:prompt, numberOfMedia:1, ...values });
const cases = [], invalid = [];
function add(name, params) {
  const options = optionsFor(params.modelId);
  cases.push({ name, params, options, wire:create('UPSTREAM-5-66', params, options) });
}
function reject(name, params) {
  const options = optionsFor(params.modelId);
  try {
    create('UPSTREAM-5-66', params, options);
  } catch (error) {
    invalid.push({ name, params, options, message:error.message });
    return;
  }
  throw new Error(`Expected upstream rejection for ${name}`);
}
for (const [index, modelId] of moss.entries()) {
  const settings = { steps:index ? 8 : 50, guidance:index ? 1 : 3, sampler:'euler', scheduler:'sgm_uniform' };
  add(`${modelId}-settings`, image(modelId, { ...settings, width:513, height:2047 }));
  add(`${modelId}-server-defaults`, image(modelId));
  add(`${modelId}-neutral-scales`, image(modelId, { mossRopeScaleX:1, mossRopeScaleY:1 }));
  add(`${modelId}-zero-and-fractional-scales`, image(modelId, { mossRopeScaleX:0, mossRopeScaleY:0.5 }));
  add(`${modelId}-maximum-scales`, image(modelId, { mossRopeScaleX:100, mossRopeScaleY:100 }));
  add(`${modelId}-personal-loras`, image(modelId, { ...settings, loras:['personal-owned-adapter'], loraStrengths:[0.7] }));
  add(`${modelId}-empty-context`, image(modelId, { contextImages:[] }));
  for (const dimension of [512, 2048]) add(`${modelId}-size-${dimension}`, image(modelId, { width:dimension, height:dimension }));
  for (const dimension of [511, 2049]) reject(`${modelId}-size-${dimension}`, image(modelId, { width:dimension, height:dimension }));
  for (const field of ['mossRopeScaleX', 'mossRopeScaleY']) {
    for (const value of [-1, 100.01, '0.5', null, true, {}]) reject(`${modelId}-${field}-${JSON.stringify(value)}`, image(modelId, { [field]:value }));
  }
  reject(`${modelId}-starting-image`, image(modelId, { startingImage:true }));
  reject(`${modelId}-context-images`, image(modelId, { contextImages:[true] }));
}
add('flux-settings-and-boundaries', image(flux, { steps:16, guidance:1, sampler:'euler', scheduler:'simple', width:256, height:4096 }));
for (const count of [0, 1, 10]) add(`flux-context-${count}`, image(flux, { contextImages:Array(count).fill(true) }));
add('flux-starting-image', image(flux, { startingImage:true, startingImageStrength:0.25 }));
for (const dimension of [255, 4097]) reject(`flux-size-${dimension}`, image(flux, { width:dimension, height:1024 }));
for (const modelId of [flux, 'flux1-schnell-fp8']) reject(`${modelId}-moss-only-scale`, image(modelId, { mossRopeScaleX:0 }));

async function capture() {
  const admission = [];
  const api = Object.create(Projects.prototype);
  for (const count of [0, 1, 10, 11]) {
    const uploads = [];
    api.uploadContextImage = async (_id, index) => { uploads.push(index); };
    const params = image(flux, { contextImages:Array(count).fill(true) });
    let message = null;
    try {
      await api._processImageAssets({ id:'UPSTREAM-5-66' }, { ...params, contextImages:Array.from({ length:count }, () => Buffer.from('synthetic-reference')) });
    } catch (error) {
      message = error.message;
    }
    admission.push({ name:`flux-upload-context-${count}`, params, accepted:message === null, message, uploadedSlots:uploads });
  }
  const lowLevelOverflow = image(flux, { contextImages:Array(11).fill(true) });
  const toolModels = [flux, 'gpt-image-2', 'krea2_identity_edit_v1_2'].map(id => ({ id, media:'image', workerCount:1 }));
  const tools = new ChatTools({ waitForModels:async () => toolModels });
  tools.executeProject = async (_call, _media, _model, params) => params;
  const png = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6c0AAAAASUVORK5CYII=';
  const toolRefusals = [];
  for (const [model, count] of [[flux, 11], ['gpt-image-2', 17], ['krea2_identity_edit_v1_2', 3]]) {
    const args = { prompt:'keep all references', model, source_image_url:png, reference_image_urls:Array(count - 1).fill(png) };
    try {
      await tools.executeImageEdit({ id:'fixture', function:{ name:'edit_image' } }, args, {});
      throw new Error(`Expected edit reference-count rejection for ${model}`);
    } catch (error) {
      if (!error.message.includes('accepts at most')) throw error;
      toolRefusals.push({ tool:'edit_image', args, message:error.message });
    }
  }
  const capabilities = [...moss, flux, 'gpt-image-2', 'krea2_identity_edit_v1_2', 'qwen_image_edit_2511_fp8', 'flux1-dev-kontext_fp8_scaled', 'flux1-schnell-fp8'].map(modelId => ({ modelId, maxContextImages:getMaxContextImages(modelId), comfy:isComfyModel(modelId) }));
  const target = path.join(__dirname, '../../src/projects/wire/fixtures/upstream-5-66.json');
  fs.writeFileSync(target, JSON.stringify({
    version, commit, protocolVersion, capabilities, cases, invalid, admission, toolModels, toolRefusals,
    lowLevelOverflow:{ params:lowLevelOverflow, wire:create('UPSTREAM-5-66', lowLevelOverflow, optionsFor(flux)) }
  }, null, 2) + '\n');
  console.log(`Captured ${cases.length} image wires, ${invalid.length} refusals, ${admission.length} upload-admission cases and ${toolRefusals.length} tool refusals from ${version} (${commit}).`);
}
capture().catch(error => { console.error(error); process.exitCode = 1; });
