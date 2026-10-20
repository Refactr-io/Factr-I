import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { expect, it } from 'vitest'

import { registerLayoutPresets } from '@/app/contrib/layout-presets'
import { $activePresetId } from '@/components/pane-shell/tree/store'
import { I18nProvider } from '@/i18n'
import { $interfaceMode } from '@/store/interface-mode'

import { LayoutPicker } from './layout-picker'

it('renders the advanced presets and applies Terminal below when clicked', () => {
  const dispose = registerLayoutPresets()
  $interfaceMode.set('advanced')
  $activePresetId.set('default')

  try {
    render(
      <I18nProvider configClient={null} initialLocale="en">
        <LayoutPicker />
      </I18nProvider>
    )

    for (const name of ['Default', 'Tools open', 'Terminal below', 'Tools left', 'Grid']) {
      expect(screen.getByRole('button', { name })).toBeTruthy()
    }

    fireEvent.click(screen.getByRole('button', { name: 'Terminal below' }))
    expect($activePresetId.get()).toBe('terminal-below')
  } finally {
    cleanup()
    dispose()
  }
})
