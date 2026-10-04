import type { SidebarNavItem } from '../../types'

/** Whether a nav item is the current page. Shared by the list rows and the rail. */
export function isNavItemActive(item: SidebarNavItem, currentView: string, pathname: string): boolean {
  return (
    (item.id === 'capabilities' && currentView === 'capabilities') ||
    (item.id === 'messaging' && currentView === 'messaging') ||
    (item.id === 'artifacts' && currentView === 'artifacts') ||
    (item.id === 'cron' && currentView === 'cron') ||
    // Contributed rows light up at their own route.
    (currentView === 'extension' && Boolean(item.route) && pathname === item.route)
  )
}
