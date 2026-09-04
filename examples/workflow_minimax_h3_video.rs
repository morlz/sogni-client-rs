//! MiniMax H3 video generation: T2V, I2V/L2V, FLF2V, and Ref2VA R2V.
//!
//! H3 generates video and native 32 kHz stereo audio together. Text-to-video,
//! image-to-video, last-frame-only video, and first/last-frame video use the
//! FL2VA family. Multi-reference R2V uses the separate Ref2VA family, where
//! labelled media conditions the result instead of acting as frame anchors.
//!
//! # Engines and fixed sampling contract
//!
//! All variants generate at 24 fps with guidance 1 and the `simple` scheduler.
//! Their remaining fixed choices are:
//!
//! - Standard: 20 steps and `res_multistep`.
//! - Balanced: 8 steps and Euler. FL2VA uses the LightX2V-derived engine;
//!   Ref2VA uses its corresponding balanced adapter.
//! - FL2VA Turbo: 4 LightX2V steps with a server-selected compatible sampler.
//! - FastH3 Turbo: a separate FastVideo VSA four-step Euler engine for T2V,
//!   I2V, and FLF2V; it has no R2V mode.
//! - Ref2VA Turbo: its dedicated four-step Euler adapter and a 960x544 default.
//!
//! Frames lie on `124 + n*17`, from 124 through 362 (about 5.17-15.08
//! seconds). Dimensions are positive multiples of 32. The standard canvas is
//! capped at 1,032,192 pixels and defaults to 1344x768; Ref2VA Turbo is capped
//! at 522,240 pixels and defaults to 960x544. Availability still depends on
//! compatible live capacity. Sogni's open-weights H3 path is 768p-class;
//! MiniMax's hosted-only 2K stage is not part of this worker example.
//!
//! Model provenance:
//!
//! - LightX2V balanced source: <https://huggingface.co/lightx2v/Minimax-h3-Turbo/tree/f3d9da6dac47dcb985684ca150f02893f619a171>
//! - Ref2VA balanced source: <https://huggingface.co/larryvrh/MiniMax-H3-Turbo-Lora/tree/7b7ac96b0616100db75ea285090210c3ddf37c04>
//!
//! # Context-IR prompting
//!
//! MiniMax describes Context-IR as important to H3 quality. The pinned official
//! guide is <https://github.com/MiniMax-AI/MiniMax-H3/tree/d21241f0a4b3acbb34c97dae47fa417b7065e438/skills/h3-prompt-writing>.
//!
//! T2VA, I2VA, L2VA, and FL2VA use these fields in this exact order:
//!
//! ```text
//! integrated_multimodal_description: [Shot 1] ...
//! overall_soundscape: ...
//! non_diegetic_music: ...
//! ```
//!
//! I2VA, L2VA, and FL2VA additionally require their mode-specific alignment
//! instruction as the first line, followed by one blank line. `[Shot 1]` has no
//! timestamp; later cuts begin `[Shot N] At MM:SS.mmm, ...`. Keep speaker IDs
//! stable as `(S1)`, `(S2)`, and so on. Preserve supplied dialogue exactly in
//! `<d>[Language] ...</d>`; invent a concise line only when speech was requested
//! without words. `overall_soundscape` contains ambience, action, and non-verbal
//! sound, never dialogue or music. `non_diegetic_music` describes audience-only
//! score, or is `N/A` when none is wanted. H3 has no negative-prompt field, so
//! exclusions belong in the positive prompt.
//!
//! A first image is I2VA; an end image alone is L2VA; supplying both endpoints
//! uses the FL2VA alignment contract. `--print-prompt` prints the exact assembled
//! prompt without authenticating or submitting work.
//!
//! # Ref2VA references
//!
//! R2V requires at least one visual reference (image or video); audio alone is
//! invalid. It accepts up to 9 images, 3 videos, 3 audio clips, and 12 files in
//! total. Use stable `<Picture N>`, `<Video N>`, `<Audio N>`, and `<Subject N>`
//! meanings. Its six sections must appear in this exact order:
//!
//! ```text
//! subject_definitions:
//! summary:
//! retention_analysis:
//! detailed_description:
//! overall_soundscape:
//! non_diegetic_music:
//! ```
//!
//! Reference-video soundtracks receive audio ordinals before standalone audio
//! references. Live R2V therefore probes every video and requires an explicit
//! `--source-audio-policy reuse|reference|replace` whenever source audio exists.
//! `reuse` requires exactly one source soundtrack and remuxes that unchanged
//! signal into the downloaded result while preserving the generated-audio file.
//! Reference videos must be 24 fps and 2-15 seconds; video and standalone-audio
//! totals are each limited to 15 seconds. FFprobe is required for live R2V, and
//! FFmpeg is required for exact soundtrack reuse.
//!
//! # Safety and credentials
//!
//! `--help`, `--print-prompt`, and `--dry-run` are credential-free and perform no
//! upload or generation. Live work requires Sogni credentials, the `fast`
//! network, explicit `--execute`, valid local media, and confirmation of the
//! server-provided estimate. `--no-audio` removes generated audio from output.
//!
//! # Usage
//!
//! ```text
//! cargo run --example workflow_minimax_h3_video -- --help
//! cargo run --example workflow_minimax_h3_video -- --mode t2v --dry-run
//! cargo run --example workflow_minimax_h3_video -- --mode flf2v --image start.jpg --end-image finish.jpg --print-prompt
//! cargo run --example workflow_minimax_h3_video -- --mode i2v --image start.jpg --execute
//! cargo run --example workflow_minimax_h3_video -- --mode r2v --ref-image face.jpg --ref-video motion.mp4 --source-audio-policy reference --execute
//! ```
//!
//! Generated MP4 files are downloaded beneath `--output` (default `output/`).

#[path = "workflow_minimax_h3_video/app.rs"]
mod app;
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
