import App from "./App.svelte";
import "./style.css";
import { defineCapsemElements } from "./elements";
import { mount } from "svelte";

defineCapsemElements();

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
