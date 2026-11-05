# Codex for cortecs.ai

This is a fork of [OpenAI Codex CLI](https://github.com/openai/codex) that works as a
terminal coding agent for [cortecs.ai](https://cortecs.ai), the EU-hosted LLM router.
It is set up for cortecs out of the box and does not send telemetry or other data to OpenAI.

## Install

You need a Rust toolchain ([rustup.rs](https://rustup.rs)) and `git`.

```sh
git clone <this repository> cortecs-codex && cd cortecs-codex
cortecs/install.sh            # builds and installs ~/.local/bin/cortecs-codex
```

The first release build takes several minutes. Use `--prefix DIR` to install somewhere
else, or `--debug` for a faster unoptimized build.

## Get started

```sh
cortecs-codex                 # first run asks for your cortecs.ai API key
```

Create an API key in the [cortecs.ai console](https://cortecs.ai). The key is stored in
`~/.cortecs-codex/auth.json`. You can provide it in other ways too:

```sh
cortecs-codex login                                        # interactive, hidden input
printenv CORTECS_API_KEY | cortecs-codex login --with-api-key
export CORTECS_API_KEY=...                                 # takes precedence over the stored key
```

Run a one-shot task without the TUI with `cortecs-codex exec "…"`.

## Models

`/model` lists the cortecs models that support tool calling, with prices (per 1M input/output
tokens), context size, and whether the model supports reasoning and images. The list
comes live from `https://api.cortecs.ai/v1/models`. The default model is `glm-5.3-flash`.
To change it, pick another model in `/model` or set it in `~/.cortecs-codex/config.toml`:

```toml
model = "glm-5.3"
model_reasoning_effort = "high"   # low | medium | high (cortecs maps it to what the model supports)
```

`-m <model>` selects a model for a single run. Models that are not in the picker, such as
models without tool calling, can still be selected by name.

## What differs from upstream Codex

| Area | Behavior in this fork |
| --- | --- |
| Provider | Built-in `cortecs` provider (Responses API at `https://api.cortecs.ai/v1`) is the default. A `[model_providers.cortecs]` table in config, as in the cortecs docs, replaces it. `CORTECS_BASE_URL` overrides the endpoint. |
| Login | API key only (no ChatGPT sign-in). `CORTECS_API_KEY` is honored. |
| Model catalog | Built from cortecs `/v1/models` (pricing, context window, reasoning, vision); OpenAI's bundled model list is never used. |
| Hosted tools | OpenAI-hosted `web_search` and image generation are not offered (cortecs does not provide them). |
| Privacy | No Statsig metrics, no analytics events, no announcement/update checks against GitHub/npm, no curated plugin sync from GitHub/chatgpt.com, `/feedback` uploads disabled. Codex turn metadata (workspace paths, git state, installation id) is not sent to cortecs. |
| Paths & name | Command `cortecs-codex`, config in `~/.cortecs-codex` (override with `CODEX_HOME`), so it can coexist with a stock Codex install. Project-level `.codex/` and `AGENTS.md` files work as usual. |
| Daemon | The background app-server daemon is off by default; the app server runs in-process. |
| Costs | `/status` shows a `Cost` line: the sum of the per-request cost reported by cortecs since `cortecs-codex` started (all threads, including sub-agents and compaction). |
| Thread titles | No automatic model-generated thread titles (they cost an extra request per thread). `/rename` still works. |

You can turn things back on in `config.toml`, for example `[features] plugins = true` or
`[feedback] enabled = true`. Feedback uploads go to OpenAI's Sentry.

## Tips

- `/status` shows the model, provider, token usage, and the cost reported by cortecs so far.
  Budgets and detailed usage are in the cortecs console.
- cortecs caches prompts automatically where the provider supports it. Long sessions get
  cache hits on the shared prefix, which lowers cost.
- Cheaper models are faster but weaker at long agentic tasks. If a model struggles,
  try a larger one (`glm-5.3`, `kimi-k2.7-code`, `deepseek-v4.1-flash`).

## Syncing with upstream

The fork keeps its changes small and well separated so you can merge upstream regularly:

```sh
git remote add upstream https://github.com/openai/codex.git   # once
git fetch upstream
git checkout main && git merge --ff-only upstream/main        # main mirrors upstream
git checkout cortecs && git rebase main                       # or: git merge main
```

Most fork-specific code is in files that upstream does not have:
`codex-rs/model-provider/src/cortecs.rs`, `codex-rs/protocol/src/provider_cost.rs` and
`cortecs/`. The rest is small, commented hooks (search the code for `cortecs`).

Tests that check upstream OpenAI defaults are marked
`#[ignore = "cortecs fork: ..."]`. If an upstream merge adds more tests like that, mark
them the same way (`grep -rn "cortecs fork:"` lists them all).
