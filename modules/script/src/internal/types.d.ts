/** Private VM bootstrap contracts; no JavaScript handles leave the owning thread. */
export type RecordValue = Record<PropertyKey, unknown>;
export interface Intrinsics {
  descriptor: typeof Object.getOwnPropertyDescriptor;
  ownKeys: typeof Reflect.ownKeys;
  define: typeof Object.defineProperty;
  freeze: typeof Object.freeze;
  seal: typeof Object.seal;
  create: typeof Object.create;
  setProto: typeof Object.setPrototypeOf;
  proto: typeof Object.getPrototypeOf;
  plain: object;
  arrayProto: object;
  isArray: (value: unknown) => value is unknown[];
  isProxy: (value: unknown) => boolean;
  stringify: typeof JSON.stringify;
  finite: typeof Number.isFinite;
  safe: typeof Number.isSafeInteger;
  integer: typeof Number.isInteger;
  toNumber: typeof Number;
  testIndex: (value: string) => boolean;
  isError: (value: object) => boolean;
  errorStack: (value: object) => unknown;
  errorPrototypes: object[];
  errorNames: string[];
  wellFormed: (value: string) => boolean;
  newWeakMap: <T>() => WeakMap<object, T>;
  weakGet: <T>(map: WeakMap<object, T>, key: object) => T | undefined;
  weakSet: <T>(map: WeakMap<object, T>, key: object, value: T) => void;
  newSet: () => Set<object>;
  setHas: (set: Set<object>, value: object) => boolean;
  setAdd: (set: Set<object>, value: object) => void;
  setDelete: (set: Set<object>, value: object) => void;
  fail: () => never;
  read: (value: unknown, key: PropertyKey) => unknown;
  record: (value: unknown) => RecordValue;
  deepFreeze: (value: unknown) => unknown;
}
export interface HeaderField { name: string; value: string }
export interface HeaderApi {
  getAll(name: unknown): string[];
  append?: (name: unknown, value: unknown) => void;
  set?: (name: unknown, value: unknown) => void;
  delete?: (name: unknown) => void;
}
export interface HeaderDraft { api: HeaderApi; snapshot(): HeaderField[] }
export type HeaderFactory = (input: unknown, writable: boolean, readOnly?: boolean) => HeaderDraft;
export interface ObservationBatch { events: number[]; dropped: number }
export interface ObservationDraft {
  api: { record(name: unknown): void };
  snapshot(): ObservationBatch;
}
export type ObservationFactory = () => ObservationDraft;
export interface RejectionDraft {
  reject(status?: unknown): void;
  snapshot(): { status: number } | undefined;
}
export type RejectionFactory = () => RejectionDraft;
export interface EventDraft { replace(replacement: unknown): void; snapshot(): unknown[] }
export type EventFactory = (root: RecordValue) => EventDraft;
export interface PhasePolicy {
  eventHook: boolean;
  observer: boolean;
  headers: boolean;
  reject: boolean;
  writable(key: PropertyKey): boolean;
}
export type PhaseFactory = (phase: unknown) => PhasePolicy;
export type Hook = (...args: unknown[]) => unknown;
export interface HookCapture { hooks: Hook[]; slots: WeakMap<object, RecordValue> }
export type Encode = (root: unknown, slots?: HookCapture["slots"]) => string;
export interface ContextDependencies {
  headers: HeaderFactory;
  observations: ObservationFactory;
  rejection: RejectionFactory;
  events: EventFactory;
  phase: PhaseFactory;
}
