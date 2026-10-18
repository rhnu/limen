export const token = limen.env("LIMEN_PROBE_ENV_TOKEN");
export const frozen =
  Object.isFrozen(limen) && !Reflect.defineProperty(limen, "moduleExtra", { value: true });
