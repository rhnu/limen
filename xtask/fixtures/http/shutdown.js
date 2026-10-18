import { defineGateway } from "limen:gateway";
import { configurePending } from "./pending.js";

export default defineGateway((gateway) => {
  configurePending(gateway, true);
});
