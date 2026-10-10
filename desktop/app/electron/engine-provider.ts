/**
 * The provider flags the desktop starts the bundled engine with.
 *
 * Only an explicit launch choice (FACTR_PROVIDER / FACTR_MODEL) becomes a flag. Without one the
 * engine decides: the pick saved in config.yaml, else the signed-in provider. A flag the desktop
 * invented would outrank the user's saved pick, so none is ever invented here.
 *
 * A value the engine's `--provider` flag would reject (clap exits before the port is announced,
 * and the user sees "invalid variant") is NOT turned into a flag: the engine reads FACTR_PROVIDER
 * from the environment itself, accepts catalog aliases there too, and for a value it cannot serve
 * starts with no provider and a readable "not supported" message in the app.
 */

/** Every value the engine's `--provider` flag accepts (ProviderChoice names and aliases in
 *  engine/src/factr_runtime.rs; engine-provider.test.ts keeps this list in step with it). */
export const ENGINE_PROVIDER_FLAG_VALUES: ReadonlySet<string> = new Set([
  '302.ai', '302ai', 'ai-studio', 'ai302', 'alibaba-coding', 'alibaba-coding-plan', 'aliyun-bailian', 'anthropic',
  'anthropic-api', 'anthropic-key', 'aoai', 'auto', 'aws-bedrock', 'aws_bedrock', 'azure', 'azure-openai', 'bailian',
  'baseten', 'bedrock', 'belvedir', 'belvedir-ai', 'belvedir.ai', 'celeris', 'celeris-1', 'celeris-ai', 'celeris1',
  'cerberascode', 'cerebras', 'cerebrascode', 'cgc', 'chatgpt', 'chatgpt-codex', 'chutes', 'claude', 'claude-api',
  'claude-code', 'claude-key', 'claude-subprocess', 'coding-plan', 'compat', 'comtegra', 'comtegra-gpu-cloud',
  'conifer', 'conifer-api', 'copilot', 'cortecs', 'cursor', 'custom', 'deep-infra', 'deepinfra', 'deepseek',
  'fireworks', 'fireworks-ai', 'fireworks.ai', 'firmware', 'fpt', 'fpt-ai', 'fpt-cloud', 'fptcloud', 'gemini',
  'gemini-api', 'gemini-apikey', 'gemini-key', 'google', 'google-ai-studio', 'grok', 'grok-build', 'groq', 'hf',
  'hugging-face', 'huggingface', 'kimi', 'kimi-code', 'kimi-coding', 'kimi-coding-plan', 'kimi-for-coding',
  'lm-studio', 'lmstudio', 'meta', 'meta-ai', 'meta-model-api', 'meta-muse', 'mimo', 'minimax', 'minimax-ai',
  'minimaxi', 'mistral', 'mistralai', 'moonshot', 'moonshot-ai', 'moonshot-coding', 'moonshotai', 'muse',
  'muse-spark', 'nebius', 'nim', 'novita', 'novita-ai', 'novita.ai', 'nvidia', 'nvidia-nim', 'ollama', 'openai',
  'openai-api', 'openai-apikey', 'openai-codex', 'openai-compatible', 'openai-key', 'openai-platform', 'opencode',
  'opencode-go', 'opencode-zen', 'opencodego', 'openrouter', 'orca-router', 'orcarouter', 'perplexity', 'pplx',
  'scaleway', 'stackit', 'together', 'together-ai', 'togetherai', 'x-ai', 'x.ai', 'xai', 'xiaomi', 'xiaomi-mimo',
  'xiaomi-mimo-api', 'yolo-auto', 'z-ai', 'z.ai', 'zai', 'zai-coding', 'zen'
])

/** Whether `value` is accepted by the engine's `--provider` flag (case-insensitive, like clap). */
export function isEngineProviderFlagValue(value: string): boolean {
  return ENGINE_PROVIDER_FLAG_VALUES.has(value.trim().toLowerCase())
}

/** A model id that is safe as a `--model` argument: no leading dash, no whitespace or control characters. */
function isModelFlagValue(value: string): boolean {
  return value.length > 0 && !value.startsWith('-') && !/[\s\p{Cc}]/u.test(value)
}

export function engineProviderArgs(env: Record<string, string | undefined>): string[] {
  const provider = env.FACTR_PROVIDER?.trim() || ''
  const model = env.FACTR_MODEL?.trim() || ''

  return [
    ...(provider && isEngineProviderFlagValue(provider) ? ['--provider', provider] : []),
    ...(model && isModelFlagValue(model) ? ['--model', model] : [])
  ]
}
