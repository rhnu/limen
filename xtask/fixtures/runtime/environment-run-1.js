import { env } from "limen:runtime";
import * as runtime from "limen:runtime";
export const token = env("LIMEN_PROBE_ENV_TOKEN");
export const sealed =
  Object.isSealed(runtime) && !Reflect.defineProperty(runtime, "moduleExtra", { value: true });
