import { capCompletionTokens, dropRequestFields, forwardClientHeader } from "limen:policies";
import { replaceText, syntheticTextTool } from "limen:content";
const fields = ["store", "metadata"];
const options = { name: "drop", fields, onChange: "changed" };
const drop = dropRequestFields(options);
fields[0] = "messages";
fields.push("stream");
options.onChange = "later";
const capOptions = { name: "cap", max: 16 };
const cap = capCompletionTokens(capOptions);
capOptions.max = 1;
const steps = [
  cap,
  drop,
  forwardClientHeader({ name: "header", from: "X-Profile", to: "x-upstream" }),
  replaceText({ name: "replace", search: "abc", replacement: "x", maxBufferedChars: 2 }),
  syntheticTextTool({
    name: "tool",
    prompt: "Return text through the declared tool.",
    injectionDepth: 2,
    maxBufferedChars: 512,
  }),
];
export default () => ({
  steps,
  frozen:
    steps.every((step) => Object.isFrozen(step) && Object.isFrozen(step.builtin)) &&
    Object.isFrozen(drop.builtin.fields),
});
