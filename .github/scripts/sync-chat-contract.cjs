// Regenerate public chat data from a freshly built sibling TypeScript SDK.
// Usage: node .github/scripts/sync-chat-contract.cjs ../sogni-client
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { createRequire } = require('node:module');
const { execFileSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const upstream = path.resolve(process.argv[2] || '../sogni-client');
const root = path.resolve(__dirname, '../..');
const compiled = path.join(upstream, 'dist/Chat/modelRouting.js');
const context = { exports: {}, require: createRequire(compiled) };
vm.runInNewContext(fs.readFileSync(compiled, 'utf8') + `
globalThis.tables = {
  image: IMAGE_MODEL_SELECTORS,
  edit: EDIT_IMAGE_MODEL_SELECTORS,
  textVideo: TEXT_VIDEO_MODEL_SELECTORS,
  imageVideo: IMAGE_VIDEO_MODEL_SELECTORS,
  videoToVideo: VIDEO_TO_VIDEO_MODEL_SELECTORS,
  soundToVideo: SOUND_TO_VIDEO_MODEL_SELECTORS
};`, context, { filename: compiled });
const routing = context.exports;
const manifest = require(path.join(upstream, 'dist/Chat/_hostedToolsManifest.generated.js')).SOGNI_HOSTED_TOOLS_MANIFEST;
const version = require(path.join(upstream, 'package.json')).version;
const commit = process.env.SOGNI_UPSTREAM_COMMIT || execFileSync('git', ['-C', upstream, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
if (!/^[0-9a-f]{40}$/.test(commit)) throw new Error('An exact upstream commit is required.');
const write = (relative, value) => {
  const target = path.join(root, relative);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, JSON.stringify(value, null, 2) + '\n');
};
write('data/hosted_tools.json', manifest);
write('data/chat_model_routing.json', { version, commit, selectors: context.tables, preferred: routing.PREFERRED_MODEL_IDS });

const selectors = [];
for (const [table, tool, extra] of [
  ['image', 'generate_image', {}], ['edit', 'edit_image', {}],
  ['textVideo', 'generate_video', {}], ['imageVideo', 'generate_video', { referenceImageIndices: [0] }],
  ['imageVideo', 'animate_photo', {}], ['videoToVideo', 'video_to_video', {}],
  ['soundToVideo', 'sound_to_video', {}]
]) {
  const key = ['generate_image', 'edit_image'].includes(tool) ? 'model' : 'videoModel';
  for (const name of [...Object.keys(context.tables[table]), 'unknown-model']) {
    for (const value of [name, `  ${name.toUpperCase().replaceAll('-', '_')}  `]) {
      const args = { ...extra, [key]: value };
      selectors.push({ tool, args, expected: routing.resolveHostedToolModelSelector(tool, args) ?? null });
    }
  }
}
const modelIds = [...new Set(Object.values(routing.PREFERRED_MODEL_IDS).flatMap(Object.values))];
const models = modelIds.map(id => ({ id, media: id.includes('tts') || id.startsWith('ace_step') || id === 'minimax_music3' ? 'audio' : Object.values(routing.PREFERRED_MODEL_IDS.video).includes(id) ? 'video' : 'image', workerCount: 1 }));
const workflows = ['upscale', 't2v', 'i2v', 'flf2v', 'r2v', 'ia2v', 'a2v', 'flfa2v', 's2v', 'v2v', 'animate-move'];
function stable(value) {
  if (Array.isArray(value)) return value.map(stable);
  if (!value || typeof value !== 'object') return value;
  return Object.fromEntries(Object.keys(value).sort().map(key => [key, stable(value[key])]));
}
const schemas = manifest.tools.map(tool => ({
  name: tool.function.name,
  sha256: createHash('sha256').update(JSON.stringify(stable(tool.function.parameters))).digest('hex')
}));
write('src/chat/fixtures/upstream-routing.json', {
  version, commit, selectors, models,
  workflows: workflows.map(workflow => ({ workflow, expected: routing.filterVideoModelsByWorkflow(models, [workflow]) })),
  defaults: [...modelIds, 'seedance-2-0-fast', 'unknown-model'].map(id => ({ id, expected: routing.getVideoDefaults(id) })),
  editModels: [...modelIds, 'qwen_image_edit_2511_fp8', 'qwen_image_edit_2511_fp8_lightning', 'unknown-model'].map(id => ({ id, expected: routing.isEditImageModel(id) })),
  schemas
});
console.log(`Synced ${manifest.tools.length} hosted tools and ${selectors.length} selector fixtures from ${version} (${commit}).`);

async function requestFixtures() {
  const ChatTools = require(path.join(upstream, 'dist/Chat/ChatTools.js')).default;
  const available = [...models, { id: 'qwen_image_edit_2511_fp8', media: 'image', workerCount: 2 }];
  const api = new ChatTools({ waitForModels: async () => available });
  api.executeProject = async (_call, _media, _model, params) => params;
  const png = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6c0AAAAASUVORK5CYII=';
  const wav = 'data:audio/wav;base64,' + Buffer.from('RIFF\0\0\0\0WAVE', 'binary').toString('base64');
  const mp4 = 'data:video/mp4;base64,' + Buffer.from([0,0,0,12,...Buffer.from('ftypisom')]).toString('base64');
  const cases = [
    ['generate_image', 'executeImageGeneration', { prompt:'a lighthouse', model:'sunburst', gpt_image_quality:' HIGH ', gpt_image_background:'TRANSPARENT', gpt_image_output_compression:0, output_format:'jpeg', width:1024, height:1024, numberOfVariations:3, seed:0 }],
    ['edit_image', 'executeImageEdit', { prompt:'a portrait', model:'flare', source_image_url:png, mask_image_url:'https://cdn.sogni.ai/mask.png', outputFormat:'webp', gptImageBackground:'opaque', gptImageOutputCompression:90 }],
    ['generate_video', 'executeVideoGeneration', { prompt:'ocean', videoModel:'seedance2-5', duration:4, outputFormat:'webm', returnLastFrame:true, generateAudio:false }],
    ['generate_video', 'executeVideoGeneration', { prompt:'walk', videoModel:'minimax-h3-fasth3-turbo-2stage', reference_image_url:png, duration:6, outputFormat:'mp4', returnLastFrame:false }],
    ['generate_video', 'executeVideoGeneration', { prompt:'scene', videoModel:'seedance2-5', referenceImageIndices:[-1], referenceVideoIndices:[0], referenceAudioIndices:[0], returnLastFrame:true }, { mediaContext:{ uploadedImages:['https://cdn.sogni.ai/a.png'], videos:['https://cdn.sogni.ai/a.mp4'], audio:['https://cdn.sogni.ai/a.wav'] } }],
    ['sound_to_video', 'executeSoundToVideo', { prompt:'sing', videoModel:'minimax-h3-fasth3-flfa2v-turbo-2stage', reference_audio_url:wav, reference_image_url:png, reference_image_end_url:png, duration:7, audio_start:2, outputFormat:'webm', returnLastFrame:true }],
    ['sound_to_video', 'executeSoundToVideo', { prompt:'sing', videoModel:'ltx25-ia2v', reference_audio_url:wav, reference_image_url:png, duration:6, width:768, height:512, generateAudio:false }],
    ['video_to_video', 'executeVideoToVideo', { prompt:'redraw', videoModel:'ltx25-v2v', control_mode:'canny', reference_video_url:mp4, outputFormat:'webm', returnLastFrame:false, detailer_strength:0.4 }],
    ['generate_music', 'executeMusicGeneration', { prompt:'music', model:'ace_step_1.5_xl_turbo', duration:20, output_format:'mp3', timesignature:4, composer_mode:false, prompt_strength:0, creativity:0.7 }],
  ];
  async function normalized(value) {
    if (value instanceof Blob) return true;
    if (Array.isArray(value)) return Promise.all(value.map(normalized));
    if (value && typeof value === 'object') return Object.fromEntries(await Promise.all(Object.entries(value).map(async ([key, val]) => [key, await normalized(val)])));
    return value;
  }
  const requests = [];
  for (const [tool, method, args, options = {}] of cases) {
    const params = await api[method]({id:'fixture', function:{name:tool}}, args, options);
    requests.push({ tool, args, options, expected:await normalized(params) });
  }
  write('src/chat/fixtures/upstream-tool-requests.json', {version, commit, models:available, requests});
  console.log(`Captured ${requests.length} direct tool request fixtures without network submission.`);
}
requestFixtures().catch(error => { console.error(error); process.exitCode = 1; });
