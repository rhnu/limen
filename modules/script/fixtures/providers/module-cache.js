import * as providers from "limen:providers";
import { createOpenAI } from "limen:providers";
import { createOpenAI as reexported } from "./reexport.mjs";
import { createOpenAI as local } from "./providers.js";

export default () => ({
  same: providers.createOpenAI === createOpenAI && createOpenAI === reexported,
  exports: Object.keys(providers).sort(),
  local,
});
