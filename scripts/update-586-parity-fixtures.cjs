// Compile the pinned TypeScript SDK in a disposable checkout first.
// Usage: node scripts/update-586-parity-fixtures.cjs BUILT_SDK SOURCE_GIT_CHECKOUT
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');
const { execFileSync } = require('node:child_process');
const built = path.resolve(process.argv[2]);
const source = path.resolve(process.argv[3]);
const commit = 'e830ed42e6b6973e659aa98c226b05f864e6ea97';
const version = '5.86.2';
if (execFileSync('git', ['rev-parse', 'HEAD'], { cwd: source, encoding: 'utf8' }).trim() !== commit ||
    require(path.join(built, 'package.json')).version !== version ||
    require(path.join(built, 'dist/version.js')).LIB_VERSION !== version) {
  throw new Error('Rebuild the intentionally pinned source before regenerating fixtures.');
}
const write = (file, data) => fs.writeFileSync(path.join(__dirname, '..', file), JSON.stringify(data, null, 2) + '\n');
const create = require(path.join(built, 'dist/Projects/createJobRequestMessage.js')).default;
const utils = require(path.join(built, 'dist/Projects/utils/index.js'));
const options = { type: 'video', steps: { min: 1, max: 50, default: 8 },
  guidance: { min: 1, max: 1, default: 1 }, sampler: { allowed: ['euler'], default: 'euler' },
  scheduler: { allowed: [], default: null } };
const cases = [], refusals = [];
function record(name, modelId, changes, reject = false) {
  const params = { type: 'video', modelId, positivePrompt: '', numberOfMedia: 1, ...changes };
  try {
    const wire = create('UPSTREAM-5-86', params, options);
    if (reject) throw new Error(`Expected rejection: ${name}`);
    cases.push({ name, params, options, wire });
  } catch (error) {
    if (!reject || error.message.startsWith('Expected rejection')) throw error;
    refusals.push({ name, params, options, message: error.message });
  }
}
for (const model of ['seedance-2-5', 'seedance-2-5-uncensored']) {
  for (const resolution of [undefined, '1080p', '4k']) {
    record(`${model} final ${resolution ?? 'default'}`, model,
      { seedanceDraftJobId: 'fixture-draft', ...(resolution ? { seedanceDraftFinalResolution: resolution } : {}) });
  }
  for (const resolution of [null, false, 2160, '', '4K', '720p', '2160p', ' 4k']) {
    record(`${model} invalid final ${JSON.stringify(resolution)}`, model,
      { seedanceDraftJobId: 'fixture-draft', seedanceDraftFinalResolution: resolution }, true);
  }
  record(`${model} orphan final resolution`, model, { seedanceDraftFinalResolution: '4k' }, true);
  record(`${model} draft with final resolution`, model, { seedanceDraft: true, seedanceDraftFinalResolution: '1080p' }, true);
  record(`${model} both draft modes`, model,
    { seedanceDraft: true, seedanceDraftJobId: 'fixture-draft', seedanceDraftFinalResolution: '4k' }, true);
}
for (const model of ['seedance-2-0', 'seedance-2-0-fast', 'seedance-2-0-mini', 'seedance-2-5']) {
  record(`${model} narrow 480p`, model, { positivePrompt: 'A synthetic coastal landscape', width: 992, height: 432, duration: 5 });
}
for (const model of ['minimax-h3-prism-int8_r2v', 'minimax-h3-prism-int8_r2v_2stage']) {
  record(`${model} references`, model, { positivePrompt: 'Synthetic reference motion', referenceImage: true, duration: 6, width: 1344, height: 768 });
  record(`${model} wrong recipe`, model, { referenceImage: true, steps: 4 }, true);
}
const models = ['minimax-h3-prism-int8_r2v', 'minimax-h3-prism-int8_r2v_2stage', 'minimax-h3-prism-int8_r2v_2stage_unknown'].map(model => ({
  model, video: utils.isVideoModel(model), h3: utils.isMinimaxH3Model(model), prism: utils.isMinimaxH3PrismModel(model),
  keyframes: utils.isMinimaxH3KeyframeModel(model), workflow: utils.getVideoWorkflowType(model) ?? null,
}));
write('src/projects/wire/fixtures/upstream-5-86.json', { version, commit, cases, refusals, models });

const routing = require(path.join(built, 'dist/Chat/modelRouting.js'));
const routingFile = path.join(__dirname, '../src/chat/fixtures/upstream-routing.json');
const routes = JSON.parse(fs.readFileSync(routingFile, 'utf8'));
routes.version = version;
routes.commit = commit;
for (const item of routes.selectors) item.expected = routing.resolveHostedToolModelSelector(item.tool, item.args) ?? null;
for (const item of routes.defaults) item.expected = routing.getVideoDefaults(item.id);
for (const item of routes.editModels) item.expected = routing.isEditImageModel(item.id);
for (const item of routes.workflows) item.expected = routing.filterVideoModelsByWorkflow(routes.models, [item.workflow]);
const canonical = value => Array.isArray(value) ? value.map(canonical) :
  value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
const manifest = require(path.join(built, 'dist/Chat/_hostedToolsManifest.generated.js')).SOGNI_HOSTED_TOOLS_MANIFEST;
routes.schemas = manifest.tools.map(tool => ({ name: tool.function.name,
  sha256: require('node:crypto').createHash('sha256').update(JSON.stringify(canonical(tool.function.parameters))).digest('hex') }));
write('src/chat/fixtures/upstream-routing.json', routes);
write('data/hosted_tools.json', manifest);

// Reuse every upstream permit mutation and its actual compiled API verdict.
// Only randomness and the clock are fixed to make the synthetic fixture repeatable.
const file = path.join(built, 'scripts/check-account-permits.cjs');
let script = fs.readFileSync(file, 'utf8');
const replace = (before, after) => {
  if (script.split(before).length !== 2) throw new Error('Upstream permit test changed: ' + before);
  script = script.replace(before, after);
};
replace("const wallet = Wallet.createRandom();", "const wallet = new Wallet('0x' + '11'.repeat(32));\nconst fixtures = [];\nDate.now = () => 1800000000000;");
replace('const calls = [];', 'const calls = [];\n  let permit;');
replace('mutate?.(data);', 'mutate?.(data);\n        permit = JSON.parse(JSON.stringify(data));');
replace('}\n(async () => {', `  fixtures.push({ approve, testnet, provider, wallet: wallet.address, now: 1800000000,
    payload: calls[0].payload, permit, rejected });\n}\n(async () => {`);
const target = path.join(__dirname, '../src/account/fixtures/permits-5-86.json');
replace("console.log('Account permit checks passed');", `require('node:fs').writeFileSync(${JSON.stringify(target)}, JSON.stringify({version:${JSON.stringify(version)},commit:${JSON.stringify(commit)},cases:fixtures},null,2)+'\\n');\nconsole.log('Exported ' + fixtures.length + ' compiled TypeScript permit cases');`);
const mod = new Module(file, module);
mod.filename = file;
mod.paths = Module._nodeModulePaths(path.dirname(file));
mod._compile(script, file);
