import { requiredEnv } from "limen:runtime";
import * as runtime from "limen:runtime";
export const token = requiredEnv("LIMEN_PROBE_ENV_TOKEN");
export const sealed =
  Object.isSealed(runtime) && !Reflect.defineProperty(runtime, "moduleExtra", { value: true });
