// Usage: node .github/scripts/sync-generation-5-79.cjs <built TypeScript SDK>
const assert = require('node:assert/strict');
const { writeFileSync } = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const upstream = path.resolve(process.argv[2] || '../sogni-client');
const commit = 'c397d41c04d5219f7a640662f621d629b82a69a6';
const version = '5.79.0';
const sourceCommit = process.env.SOGNI_UPSTREAM_COMMIT ??
  execFileSync('git', ['-C', upstream, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
assert.equal(sourceCommit, commit);
assert.equal(require(path.join(upstream, 'dist/version.js')).LIB_VERSION, version);
assert.equal(require(path.join(upstream, 'node_modules/@sogni-ai/sogni-protocol/package.json')).version, '1.0.0-alpha.59');
const create = require(path.join(upstream, 'dist/Projects/createJobRequestMessage.js')).default;
const gif = require(path.join(upstream, 'dist/lib/gptImageGif.js'));

const draftId = 'example-draft-job';
const image = { type: 'image', modelId: 'gpt-image-2.6-flare', positivePrompt: 'A looping wave', numberOfMedia: 1 };
const animation = { ...image, outputFormat: 'gif' };
const video = { type: 'video', modelId: 'seedance-2-5', positivePrompt: 'A wave rolls in', numberOfMedia: 1 };
const draft = { ...video, seedanceDraft: true, duration: 8, width: 864, height: 496 };
const finalize = { ...video, positivePrompt: '', seedanceDraftJobId: draftId };
const selection = {
  ...video, modelId: 'sam3_video_segment_fp16', positivePrompt: '',
  referenceVideo: true, sam3VideoPrompt: { text: 'boat', frameIndex: 126 }
};
const h3 = {
  ...video, modelId: 'minimax-h3-fl2va-fp8_t2v_balanced', width: 1344, height: 768,
  duration: 124 / 24, returnLastFrame: true
};
const inputs = [
  ['gif defaults', animation],
  ['GIF geometry overrides a named preset', {
    ...animation, modelId: 'gpt-image-2.6-sunburst', sizePreset: 'portrait',
    width: 320, gptImageFrameCount: 8, gptImageFrameDurationMs: 200, gptImageComplexity: 'medium'
  }],
  ['GIF with ordered references', { ...animation, width: 320, height: 320, contextImages: [true, true] }],
  ['GIF with a named preset', { ...animation, sizePreset: 'portrait', gptImageFrameCount: 16 }],
  ['markup still edit', {
    ...image, gptImageMarkupUrl: 'https://example.test/overlay.png', gptImageComplexity: 'low',
    contextImages: [true], outputFormat: 'webp'
  }],
  ['draft', draft],
  ['draft disabled', { ...draft, seedanceDraft: false }],
  ['uncensored draft with first and last frames', {
    ...draft, modelId: 'seedance-2-5-uncensored', referenceImage: true, referenceImageEnd: true,
    seed: 4, generateAudio: false
  }],
  ['finalize', finalize],
  ['finalize with delivery options', {
    ...finalize, modelId: 'seedance-2-5-uncensored', positivePrompt: 'Ignored prompt',
    outputFormat: 'mov', returnLastFrame: true, seedanceDraft: false, referenceImageUrls: []
  }],
  ['MiniMax H3 last frame', h3],
  ['SAM 3 video source probing', selection],
  ['SAM 3 video exact source geometry', {
    ...selection, width: 1920, height: 1080, frames: 300, fps: 30000 / 1001,
    sam3VideoPrompt: { points: [{ x: 0.5, y: 0.25, label: 'positive' }], frameIndex: 299, objectIndex: 1 }
  }],
  ['Priority enabled', { ...image, priorityMode: true }],
  ['Priority disabled', { ...image, priorityMode: false }],
  ['Priority omitted', image],
];
const invalid = [
  ['animated older model', { ...animation, modelId: 'gpt-image-2.5-flare' }, 'require GPT Image 2.6'],
  ['too large for a GIF sheet', { ...animation, width: 1024, height: 1024 }, 'pixel budget'],
  ['GIF and edit mask', { ...animation, contextImages: [true], gptImageMask: true }, 'masks or markup'],
  ['GIF and markup', { ...animation, gptImageMarkupUrl: 'https://example.test/overlay.png' }, 'masks or markup'],
  ['GIF transparent background', { ...animation, gptImageBackground: 'transparent' }, 'PNG or WebP'],
  ['GIF compression', { ...animation, gptImageOutputCompression: 50 }, 'requires JPEG or WebP'],
  ['GIF frame options for stills', { ...image, gptImageFrameCount: 9 }, 'require GIF'],
  ['markup missing edit reference', { ...image, gptImageMarkupUrl: 'https://example.test/overlay.png' }, 'first edit reference'],
  ['draft wrong model', { ...draft, modelId: 'seedance-2-0' }, 'only by Seedance 2.5'],
  ['draft malformed flag', { ...draft, seedanceDraft: 'true' }, 'seedanceDraft must be a boolean'],
  ['finalize blank draft ID', { ...finalize, seedanceDraftJobId: ' ' }, 'must be the Sogni job id'],
  ['draft and finalize conflict', { ...finalize, seedanceDraft: true }, 'mutually exclusive'],
  ['finalize sends inputs', { ...finalize, duration: 8, seed: 1, generateAudio: false }, 'remove duration, seed, generateAudio'],
  ['finalize sends two outputs', { ...finalize, numberOfMedia: 2 }, 'numberOfMedia must be 1'],
  ['SAM 3 video no source', { ...selection, referenceVideo: false }, 'requires referenceVideo'],
  ['SAM 3 video two outputs', { ...selection, numberOfMedia: 2 }, 'numberOfMedia: 1'],
  ['SAM 3 video frame out of range', { ...selection, frames: 126 }, 'must refer to a source frame'],
  ['SAM 3 video soft mask', { ...selection, sam3VideoPrompt: { text: 'boat', edges: 'soft' } }, 'hard mask output'],
  ['SAM 3 video multiple objects', { ...selection, sam3VideoPrompt: { text: 'boat', maxInstances: 2 } }, 'tracks one object'],
  ['SAM 3 video trim', { ...selection, trimEndFrame: true }, 'complete source video'],
  ['SAM 3 prompt on another model', { ...video, sam3VideoPrompt: { text: 'boat' } }, 'requires the SAM 3 video selection model'],
  ['non-boolean Priority', { ...image, priorityMode: 'false' }, 'must be true or false'],
];

const options = type => ({
  type, steps: { min: 1, max: 50, default: 1 }, guidance: { min: 0, max: 20, default: 1 },
  sampler: { allowed: [], default: null }, scheduler: { allowed: [], default: null },
});
const frameFields = [
  'modelID', 'positivePrompt', 'sizePreset', 'width', 'height', 'frames', 'fps',
  'gptImageFrameCount', 'gptImageFrameDurationMs', 'gptImageComplexity', 'gptImageMarkupUrl',
  'seedanceDraft', 'seedanceDraftJobId', 'returnLastFrame', 'generateAudio', 'seed',
  'hasReferenceVideo', 'hasReferenceImage', 'hasReferenceImageEnd', 'interpolation', 'sam3VideoPrompt',
  'referenceImageURLs', 'referenceVideoURLs', 'referenceAudioURLs',
];
const select = (object, fields) => Object.fromEntries(fields.map(field => [field, object[field] ?? null]));
const wire = params => create('UPSTREAM-5-79', params, options(params.type));
const cases = inputs.map(([name, params]) => {
  const request = wire(params);
  const isGifOrMarkup = params.outputFormat === 'gif' || params.gptImageMarkupUrl;
  const fields = isGifOrMarkup ? [...frameFields, 'hasContextImage1', 'hasContextImage2'] : frameFields;
  // Existing Rust Seedance requests explicitly send their fixed 24 FPS.
  const comparableFields = params.modelId.startsWith('seedance-') ? fields.filter(field => field !== 'fps') : fields;
  return {
    name, params, request: select(request, ['outputFormat', 'numberOfImages', 'priorityMode']),
    frame: select(request.keyFrames[0], comparableFields)
  };
});
const refusals = invalid.map(([name, params, message]) => {
  assert.throws(() => wire(params), error => error.message.includes(message), name);
  return { name, params, message };
});
const sizes = [2, 8, 9, 10, 12, 13, 16].flatMap(count =>
  [1 / 3, 9 / 16, 1, 16 / 9, 3].map(ratio => {
    const { width, height } = gif.resolveGptImageGifSize(undefined, undefined, count, ratio);
    return { count, ratio, width, height, sheet: gif.getGptImageGifSheet(width, height, count) };
  })
);
writeFileSync(
  path.resolve(__dirname, '../../src/projects/wire/fixtures/upstream-5-79.json'),
  JSON.stringify({ version, commit, cases, refusals, sizes }, null, 2) + '\n'
);
console.log(`Captured ${cases.length} wire cases, ${refusals.length} refusals and ${sizes.length} GIF layouts from ${version}.`);
