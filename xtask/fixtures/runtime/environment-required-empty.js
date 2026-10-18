import { requiredEnv } from "limen:runtime";
const token = requiredEnv("LIMEN_PROBE_ENV_EMPTY");
export default () => ({ token });
