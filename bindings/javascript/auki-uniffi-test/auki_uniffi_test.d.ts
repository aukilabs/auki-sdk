/* tslint:disable */
/* eslint-disable */

export class Counter {
    free(): void;
    [Symbol.dispose](): void;
    addAfter(delta: number, delay_ms: number): Promise<any>;
    constructor(initial: number);
    value(): number;
}

export class Greeting {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    readonly message: string;
    readonly nameLength: number;
    readonly style: GreetingStyle;
}

export enum GreetingStyle {
    Casual = 0,
    Formal = 1,
}

export function add(left: number, right: number): number;

export function delayedGreeting(name: string, delay_ms: number): Promise<Greeting>;

export function hello(name: string): string;

export function makeGreeting(name: string, style: GreetingStyle): Greeting;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_counter_free: (a: number, b: number) => void;
    readonly __wbg_greeting_free: (a: number, b: number) => void;
    readonly add: (a: number, b: number) => number;
    readonly counter_addAfter: (a: number, b: number, c: number) => any;
    readonly counter_new: (a: number) => number;
    readonly counter_value: (a: number) => number;
    readonly delayedGreeting: (a: number, b: number, c: number) => any;
    readonly greeting_message: (a: number) => [number, number];
    readonly greeting_nameLength: (a: number) => number;
    readonly greeting_style: (a: number) => number;
    readonly hello: (a: number, b: number) => [number, number];
    readonly makeGreeting: (a: number, b: number, c: number) => [number, number, number];
    readonly wasm_bindgen__convert__closures_____invoke__hd7378cf5d3176325: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen__convert__closures_____invoke__h019a0ecdfde4e23b: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__haecf00134c5ebb79: (a: number, b: number) => void;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_destroy_closure: (a: number, b: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
