/** The window's native menu bar. */
import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu } from '@tauri-apps/api/menu'

export const EXPORT_SCALES = [0.5, 1, 2, 4]

export interface MenuActions {
  newScene(): void
  openFile(): void
  openRecent(path: string): void
  openExample(name: string): void
  saveFile(): void
  saveFileAs(): void
  exportPng(): void
  setExportScale(scale: number): void
  quit(): void
  undo(): void
  redo(): void
  addPath(): void
  setOverlay(on: boolean): void
  setHidden(on: boolean): void
  zoomIn(): void
  zoomOut(): void
  fit(): void
  actualSize(): void
}

export interface MenuState {
  recent: string[]
  canExport: boolean
  canUndo: boolean
  canRedo: boolean
  exportScale: number
  showOverlay: boolean
  showHidden: boolean
}

/** Handles to the items whose state changes, so they can be updated in place. */
export interface AppMenu {
  update(state: MenuState): Promise<void>
}

/** Builds the menu bar, installs it, and returns a way to keep it in step with the app. */
export async function installMenu(
  actions: MenuActions,
  examples: string[],
  label: (path: string) => string,
): Promise<AppMenu> {
  const sep = () => PredefinedMenuItem.new({ item: 'Separator' })
  const item = (text: string, action: () => void, accelerator?: string) =>
    MenuItem.new({ text, action, accelerator })

  // Native check items flip themselves when clicked. Put them back as the app
  // sees things once it has reacted, in case the click changed nothing.
  const resync = () => setTimeout(() => state && sync(state))

  const undoItem = await item('Undo', actions.undo, 'CmdOrCtrl+Z')
  const redoItem = await item('Redo', actions.redo, 'CmdOrCtrl+Shift+Z')
  const recentMenu = await Submenu.new({ text: 'Open Recent', items: [] })
  const exportItem = await item('Export PNG…', actions.exportPng, 'CmdOrCtrl+Shift+E')
  const scaleItems = await Promise.all(
    EXPORT_SCALES.map((s) => CheckMenuItem.new({ text: `${s}×`, action: () => (actions.setExportScale(s), resync()) })),
  )
  const overlayItem = await CheckMenuItem.new({
    text: 'Overlay',
    accelerator: 'CmdOrCtrl+Shift+L',
    action: () => (actions.setOverlay(!state?.showOverlay), resync()),
  })
  const hiddenItem = await CheckMenuItem.new({
    text: 'Masks and Defs',
    action: () => (actions.setHidden(!state?.showHidden), resync()),
  })

  const menu = await Menu.new({
    items: [
      await Submenu.new({
        text: 'File',
        items: [
          await item('New', actions.newScene, 'CmdOrCtrl+N'),
          await item('Open…', actions.openFile, 'CmdOrCtrl+O'),
          recentMenu,
          await Submenu.new({
            text: 'Open Example',
            items: await Promise.all(examples.map((name) => item(name, () => actions.openExample(name)))),
          }),
          await sep(),
          await item('Save', actions.saveFile, 'CmdOrCtrl+S'),
          await item('Save As…', actions.saveFileAs, 'CmdOrCtrl+Shift+S'),
          await sep(),
          exportItem,
          await Submenu.new({ text: 'Export Scale', items: scaleItems }),
          await sep(),
          await item('Quit', actions.quit, 'CmdOrCtrl+Q'),
        ],
      }),
      await Submenu.new({ text: 'Edit', items: [undoItem, redoItem] }),
      await Submenu.new({
        text: 'Scene',
        items: [await item('Add Path', actions.addPath, 'CmdOrCtrl+Shift+A')],
      }),
      await Submenu.new({
        text: 'View',
        items: [
          overlayItem,
          hiddenItem,
          await sep(),
          await item('Zoom In', actions.zoomIn, 'CmdOrCtrl+='),
          await item('Zoom Out', actions.zoomOut, 'CmdOrCtrl+-'),
          await item('Fit', actions.fit, 'CmdOrCtrl+0'),
          await item('Actual Size', actions.actualSize, 'CmdOrCtrl+1'),
        ],
      }),
    ],
  })
  await menu.setAsAppMenu()

  let state: MenuState | undefined
  let shownRecent: string[] | undefined
  let queue = Promise.resolve()

  // One update at a time, so rebuilding the recent list never interleaves.
  function sync(next: MenuState) {
    state = next
    queue = queue.then(() => apply(next)).catch((e) => console.error('menu update failed', e))
    return queue
  }

  return { update: sync }

  async function apply(next: MenuState) {
    await Promise.all([
      exportItem.setEnabled(next.canExport),
      undoItem.setEnabled(next.canUndo),
      redoItem.setEnabled(next.canRedo),
      ...scaleItems.map((it, i) => it.setChecked(EXPORT_SCALES[i] === next.exportScale)),
      overlayItem.setChecked(next.showOverlay),
      hiddenItem.setChecked(next.showHidden),
      hiddenItem.setEnabled(next.showOverlay),
    ])
    if (shownRecent?.join('\n') === next.recent.join('\n')) return
    shownRecent = next.recent
    for (const old of await recentMenu.items()) {
      await recentMenu.remove(old)
      await old.close()
    }
    if (next.recent.length) {
      await recentMenu.append(
        await Promise.all(next.recent.map((p) => item(label(p), () => actions.openRecent(p)))),
      )
    } else {
      await recentMenu.append(await MenuItem.new({ text: 'No Recent Files', enabled: false }))
    }
  }
}
