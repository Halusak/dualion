import { getCurrentWindow } from '@tauri-apps/api/window';
import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import JobsWindow from './components/jobs/JobsWindow.svelte';

// The Operations window loads the same page under its own label.
const root = getCurrentWindow().label === 'jobs' ? JobsWindow : App;
const app = mount(root, { target: document.getElementById('app')! });

export default app;
