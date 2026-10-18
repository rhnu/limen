import { replaceText } from "limen:content";
const invalid = replaceText({ name: "private-value", stateKey: "removed" });
export default () => invalid;
