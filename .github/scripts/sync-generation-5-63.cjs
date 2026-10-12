// Capture the new public 5.63.1 generation contracts from the actual TS serializer.
// Usage: node .github/scripts/sync-generation-5-63.cjs ../sogni-client
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const upstream = path.resolve(process.argv[2] || '../sogni-client');
const commit = execFileSync('git', ['-C', upstream, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
const version = require(path.join(upstream, 'package.json')).version;
const builtVersion = require(path.join(upstream, 'dist/version.js')).LIB_VERSION;
if (commit !== 'e3797a2ed623689309cf28186180f866683c3569' || version !== '5.63.1' || builtVersion !== version) {
  throw new Error('Build the reviewed, pinned upstream 5.63.1 revision before generating fixtures.');
}
const create = require(path.join(upstream, 'dist/Projects/createJobRequestMessage.js')).default;
const options = type => ({ type, sampler: { allowed: [], default: null }, scheduler: { allowed: [], default: null } });
const refs = (count, extension) => Array.from({ length: count }, (_, i) => `https://cdn.example.com/ref-${i}.${extension}`);
const cases = [], invalid = [];
const video = (modelId, values = {}) => ({ type: 'video', modelId, positivePrompt: 'A kite above the garden.', numberOfMedia: 1, duration: 5, ...values });
function add(name, params) {
  const modelOptions = options(params.type);
  cases.push({ name, params, options: modelOptions, wire: create('UPSTREAM-5-63', params, modelOptions) });
}
function reject(name, params) {
  const modelOptions = options(params.type);
  try {
    create('UPSTREAM-5-63', params, modelOptions);
  } catch (error) {
    invalid.push({ name, params, options: modelOptions, message: error.message });
    return;
  }
  throw new Error(`Expected upstream rejection for ${name}`);
}
const full = 'seedance-2-5-uncensored';
const mini = 'seedance-2-0-mini-uncensored';
for (const modelId of [full, mini]) {
  for (const duration of [4, modelId === full ? 30 : 15]) add(`${modelId}-duration-${duration}`, video(modelId, { duration }));
  for (const duration of [3, modelId === full ? 31 : 16]) reject(`${modelId}-duration-${duration}`, video(modelId, { duration }));
}
add('uncensored-25-export', video(full, { outputFormat: 'mov', returnLastFrame: true }));
add('uncensored-25-first-last', video(full, { referenceImage: true, referenceImageEnd: true }));
for (const seedanceTaskType of ['edit', 'extend']) {
  add(`uncensored-25-${seedanceTaskType}`, video(full, { seedanceTaskType, referenceVideoUrls: refs(1, 'mp4') }));
  reject(`uncensored-25-${seedanceTaskType}-no-video`, video(full, { seedanceTaskType }));
}
add('uncensored-25-full-reference-budget', video(full, { seedanceTaskType: 'reference', referenceImageUrls: refs(30, 'png'), referenceVideoUrls: refs(10, 'mp4'), referenceAudioUrls: refs(10, 'mp3') }));
add('uncensored-25-audio-reference', video(full, { seedanceTaskType: 'reference', referenceAudioUrls: refs(1, 'mp3') }));
for (const [field, count, extension] of [['referenceImageUrls', 31, 'png'], ['referenceVideoUrls', 11, 'mp4'], ['referenceAudioUrls', 11, 'mp3']]) {
  reject(`uncensored-25-${field}-over-budget`, video(full, { seedanceTaskType: 'reference', [field]: refs(count, extension) }));
}
reject('uncensored-25-loose-without-task', video(full, { referenceImageUrls: refs(1, 'png') }));
reject('uncensored-25-frame-and-loose', video(full, { seedanceTaskType: 'reference', referenceImage: true, referenceImageUrls: refs(1, 'png') }));
add('uncensored-mini-image-video-budget', video(mini, { referenceImageUrls: refs(9, 'png'), referenceVideoUrls: refs(3, 'mp4') }));
add('uncensored-mini-image-audio-budget', video(mini, { referenceImageUrls: refs(9, 'png'), referenceAudioUrls: refs(3, 'mp3') }));
for (const [field, count, extension] of [['referenceImageUrls', 10, 'png'], ['referenceVideoUrls', 4, 'mp4'], ['referenceAudioUrls', 4, 'mp3']]) {
  reject(`uncensored-mini-${field}-over-budget`, video(mini, { referenceImage: true, [field]: refs(count, extension) }));
}
reject('uncensored-mini-total-budget', video(mini, { referenceImageUrls: refs(9, 'png'), referenceVideoUrls: refs(3, 'mp4'), referenceAudioUrls: refs(1, 'mp3') }));
reject('uncensored-mini-audio-alone', video(mini, { referenceAudioUrls: refs(1, 'mp3') }));
reject('uncensored-mini-task-type', video(mini, { seedanceTaskType: 'reference', referenceImageUrls: refs(1, 'png') }));
reject('uncensored-mini-mov', video(mini, { outputFormat: 'mov' }));
reject('uncensored-mini-last-frame-export', video(mini, { returnLastFrame: true }));
for (const [type, modelId] of [['image', 'flux1-schnell-fp8'], ['video', full], ['audio', 'ace_step_1.5_turbo']]) {
  for (const promptExpanded of [undefined, false, true]) {
    add(`${type}-prompt-expanded-${promptExpanded}`, {
      type, modelId, positivePrompt: 'Unchanged prompt.', numberOfMedia: 1,
      ...(promptExpanded === undefined ? {} : { promptExpanded })
    });
  }
}
const destination = path.join(__dirname, '../../src/projects/wire/fixtures/upstream-5-63.json');
fs.writeFileSync(destination, JSON.stringify({ version, commit, cases, invalid }, null, 2) + '\n');
console.log(`Captured ${cases.length} new generation wires and ${invalid.length} rejections from ${version} (${commit}).`);
