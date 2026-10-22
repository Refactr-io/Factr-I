import { describe, expect, it } from 'vitest'

import { configSubpageForField } from './config-subpages'
import { SECTIONS } from './constants'

// Where each opt-in control of 0.0.11 shows up in Settings.
const PLACEMENT: Record<string, string> = {
  'approvals.always_ask': 'safety',
  'approvals.gui_automation_ask': 'safety',
  'budget.max_cost_usd': 'advanced',
  'budget.max_tokens': 'advanced',
  'compression.deterministic': 'memory',
  'compression.llm_second_pass': 'memory',
  'local_models.silence_timeout_secs': 'advanced',
  'persistence.enabled': 'advanced',
  'persistence.max_retries': 'advanced',
  'persistence.max_tokens': 'advanced',
  'terminal.reduce_output': 'advanced',
  'terminal.reduce_output_timeout_ms': 'advanced',
  'tools.routing': 'advanced'
}

describe('opt-in controls in Settings', () => {
  for (const [key, section] of Object.entries(PLACEMENT)) {
    it(`${key} is listed under ${section} and has a page`, () => {
      expect(SECTIONS.find(s => s.id === section)?.keys).toContain(key)
      expect(configSubpageForField(section, key)).toBeTruthy()
    })
  }
})
