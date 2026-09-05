import { createApp } from "vue";
import App from "./Root.vue";
import { i18n, initializeLocale } from "./i18n";
import "./styles/base.css";
import "./styles/notes.css";
import "./styles/sticky.css";

async function bootstrap() {
  await initializeLocale();
  createApp(App).use(i18n).mount("#app");
}

void bootstrap();
