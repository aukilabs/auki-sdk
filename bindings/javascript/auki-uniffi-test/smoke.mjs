import { readFile } from "node:fs/promises";
import init, {
  add,
  hello,
  makeGreeting,
  delayedGreeting,
  Counter,
  GreetingStyle,
} from "./auki_uniffi_test.js";

const wasmBytes = await readFile(new URL("./auki_uniffi_test_bg.wasm", import.meta.url));
await init({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

assert(add(2, 3) === 5, "add failed");
assert(hello("JavaScript") === "Hello, JavaScript.", "hello failed");

const greeting = makeGreeting("JavaScript", GreetingStyle.Formal);
assert(greeting.message === "Good day, JavaScript.", "formal greeting failed");
assert(greeting.nameLength === 10, "nameLength failed");
assert(greeting.style === GreetingStyle.Formal, "greeting style failed");
if (typeof greeting.free === "function") {
  greeting.free();
}

const delayed = await delayedGreeting("JavaScript", 0);
assert(delayed.message === "Hello, JavaScript.", "delayed greeting failed");
assert(delayed.style === GreetingStyle.Casual, "delayed greeting style failed");
if (typeof delayed.free === "function") {
  delayed.free();
}

const counter = new Counter(10);
assert(counter.value() === 10, "counter initial failed");
const updated = await counter.addAfter(7, 0);
assert(updated === 17, "counter update failed");
assert(counter.value() === 17, "counter final failed");
if (typeof counter.free === "function") {
  counter.free();
}

const releasedCounter = new Counter(20);
const pendingUpdate = releasedCounter.addAfter(4, 1);
if (typeof releasedCounter.free === "function") {
  releasedCounter.free();
}
assert(await pendingUpdate === 24, "counter pending update after free failed");

console.log("javascript wasm smoke ok");
