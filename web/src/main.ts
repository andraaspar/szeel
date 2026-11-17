import { mount } from 'svelte'
import './app.css'
import { ready } from './lib/wasm'

// The app analyses the scene synchronously as it starts, so wait for the module first.
await ready
const { default: App } = await import('./App.svelte')
mount(App, { target: document.getElementById('app')! })
