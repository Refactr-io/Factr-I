import { createContext, useContext } from 'react'

/** Id of the visible label a row gives its control, so a control without its own name (a select
 *  trigger, whose combobox role takes no name from content) is named by the row title. */
export const FieldLabelContext = createContext<string | undefined>(undefined)

export const useFieldLabelId = (): string | undefined => useContext(FieldLabelContext)
