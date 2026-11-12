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

### Keep credentials away from the sandbox (recommended)

Whatever a command prints is sent to cortecs and the model provider. By default sandboxed
commands can read your whole home directory, including `~/.ssh`, `~/.aws` or the cortecs
API key. To block that, run once:

```sh
cortecs/install.sh --protect-secrets   # writes /etc/codex/requirements.toml (needs sudo)
```

This adds a `deny_read` list (SSH and GPG keys, cloud and Kubernetes credentials, `gh`,
Docker and package-registry tokens, browser profiles, Codex API keys) to the system-wide
Codex requirements. Sandboxed commands cannot read these paths in any permission mode, and
commands you approve to run with more permissions stay sandboxed for these paths too. Only
full access (no sandbox) ignores the list. It applies to all users and to any upstream Codex
on the machine. Running it again updates the list; edit the file to add your own paths.
Linux needs `bwrap` (bubblewrap) installed, which Codex also uses for its normal sandbox.

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

## Browser GUI (experimental)

```sh
cortecs-codex gui             # opens the current folder in your browser
cortecs-codex gui ~/src/app   # or another folder; --no-browser only prints the URL
```

This starts a small local web server (127.0.0.1 only, guarded by a random token in the URL)
and opens the page in your browser. Keep the terminal open; Ctrl-C quits. Each browser tab
runs its own `cortecs-codex app-server`, the same engine the TUI uses, so config, the API
key, the sandbox, approvals and `-c` overrides behave exactly as in the TUI.

- **Folders:** pick a folder with *Change…*. A folder without a saved trust decision gets the
  same prompt as in the TUI (*Trust and continue* / *Look without trusting* / *Cancel*).
- **Sessions:** the sidebar lists the saved sessions of the current folder, from the TUI and
  the GUI alike. GUI sessions are stored like TUI sessions and appear in `cortecs-codex resume`.
- **Switching between TUI and GUI:** a session can be open in only one place at a time. Opening
  it in a second place fails with "already has an active writer" until you quit it in the TUI,
  or switch to another session or close the tab in the GUI.
- **Approvals:** commands and edits that need approval open a dialog (run, allow for the
  session, decline, or decline and stop the turn).
- The model picker overrides `model` from `config.toml` only once you pick a model.
- **Slash commands:** type `/` in the message box for a list: `/new`, `/clear`, `/status`,
  `/model`, `/stop`, `/rename`, `/compact`, `/review`, `/init`, `/diff`, `/fork`, `/archive`,
  `/delete`, `/mcp`, `/skills`, `/pwd`, `/help`. `/status` includes the cost reported by cortecs
  for everything run in that browser tab. Start a message with `//` to send a literal leading slash.

## Models

`/model` lists the cortecs models that support tool calling, with prices (per 1M input/output
tokens), context size, and whether the model supports reasoning and images. The list
comes live from `https://api.cortecs.ai/v1/models`. The default model is `glm-5.3-flash`.
To change it, pick another model in `/model` or set it in `~/.cortecs-codex/config.toml`:

```toml
model = "glm-5.3"
model_reasoning_effort = "high"   # low | medium | high (cortecs maps it to what the model supports)
```

The picker starts with a few suggested models (cheap, medium, expensive): `glm-5.3-flash`,
`mistral-large-4-0`, `gemini-4-argon`. All other models follow alphabetically. To choose your
own suggestions, set them in `config.toml`. Models that cortecs does not offer, for example
with EU-only routing, are skipped:

```toml
suggested_models = ["glm-5.3-flash", "mistral-large-4-0", "gemini-4-argon"]
```

In `/model`, Enter switches the model for the current session only. Press `d` instead to
also save it as the default for new sessions (written to `config.toml`), so an expensive
model does not stick around by accident.

`-m <model>` selects a model for a single run. Models that are not in the picker, such as
models without tool calling, can still be selected by name.

## What differs from upstream Codex

| Area | Behavior in this fork |
| --- | --- |
| Provider | Built-in `cortecs` provider (Responses API at `https://api.cortecs.ai/v1`) is the default. A `[model_providers.cortecs]` table in config, as in the cortecs docs, replaces it. `CORTECS_BASE_URL` overrides the endpoint. |
| Login | API key only (no ChatGPT sign-in). `CORTECS_API_KEY` is honored. A missing or rejected key stops with a clear message instead of retrying. |
| Model catalog | Built from cortecs `/v1/models` (pricing, context window, reasoning, vision); OpenAI's bundled model list is never used. |
| Sandbox network | Sandboxed commands have network access by default (upstream blocks it), so commands like `npm install` don't need to leave the sandbox. Set `[sandbox_workspace_write] network_access = false` to turn it off. Custom `[permissions]` profiles keep their own `network` setting. |
| Model picker | Enter in `/model` applies to the current session only; `d` saves the choice as the default. Upstream saves on Enter and uses `s` for session-only. |
| Hosted tools | OpenAI-hosted `web_search` and image generation are not offered (cortecs does not provide them), and the bundled OpenAI-specific skills (`imagegen`, `openai-docs`) are removed. |
| Privacy | No Statsig metrics, no analytics events, no announcement/update checks against GitHub/npm, no curated plugin sync from GitHub/chatgpt.com, `/feedback` uploads disabled. Codex turn metadata (workspace paths, git state, installation id) is not sent to cortecs. |
| Paths & name | Command `cortecs-codex`, config in `~/.cortecs-codex` (override with `CODEX_HOME`), so it can coexist with a stock Codex install. Project-level `.codex/` and `AGENTS.md` files work as usual. |
| Daemon | The background app-server daemon is off by default; the app server runs in-process. |
| Costs | `/status` shows a `Cost` line: the sum of the per-request cost reported by cortecs since `cortecs-codex` started (all threads, including sub-agents and compaction). |
| Untrusted folders | The folder prompt has a third choice, **Look without trusting**: the folder opens for this session only, nothing is saved, and `AGENTS.md`, project skills (`.agents/skills`, `.codex/skills`), project config, hooks and exec policies are ignored. Anything other than read-only commands needs your approval. `--untrusted` does the same without the prompt, also for `exec`. Folders marked `trust_level = "untrusted"` never get project skills. |
| GUI | `cortecs-codex gui` adds an experimental browser GUI (new `codex-rs/gui` crate) on top of the app server. Upstream has no GUI in this repository. The app-server's `thread/tokenUsage/updated` notification gains an optional `providerCostMicros` field so clients can show the cortecs cost. |
| Thread titles | No automatic model-generated thread titles (they cost an extra request per thread). `/rename` still works. |

You can turn things back on in `config.toml`, for example `[features] plugins = true` or
`[feedback] enabled = true`. Feedback uploads go to OpenAI's Sentry.

## Tips

- To inspect a repository you don't trust, choose **Look without trusting** at the folder
  prompt or start with `cortecs-codex --untrusted`.
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
`codex-rs/model-provider/src/cortecs.rs`, `codex-rs/protocol/src/provider_cost.rs`,
`codex-rs/gui/` and `cortecs/`. The rest is small, commented hooks (search the code for `cortecs`).

Tests that check upstream OpenAI defaults are marked
`#[ignore = "cortecs fork: ..."]`. If an upstream merge adds more tests like that, mark
them the same way (`grep -rn "cortecs fork:"` lists them all).
