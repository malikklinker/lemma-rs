import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

export class HostClient {
  constructor(executable, args = [], options = {}) {
    this.pending = new Map(); this.nextId = 1;
    this.closed = false;
    this.process = spawn(executable, ["serve", ...args], { stdio: ["pipe", "pipe", "inherit"], ...options });
    this.lines = createInterface({ input: this.process.stdout });
    this.lines.on("line", line => {
      let response;
      try { response = JSON.parse(line); }
      catch { this.fail(new Error(`Host emitted invalid JSON: ${line.slice(0, 200)}`)); return; }
      if (!response || typeof response !== "object" || Array.isArray(response)
        || response.version !== 1 || typeof response.ok !== "boolean"
        || !Number.isSafeInteger(response.id) || !this.pending.has(response.id)
        || (response.ok ? !("result" in response) || "error" in response
          : !response.error || typeof response.error.code !== "string"
            || typeof response.error.message !== "string" || "result" in response)) {
        this.fail(new Error("Rust host emitted an invalid version 1 response")); return;
      }
      const request = this.pending.get(response.id);
      clearTimeout(request.timer); this.pending.delete(response.id);
      if (response.ok) request.resolve(response.result);
      else request.reject(Object.assign(new Error(response.error.message), { code: response.error.code }));
    });
    this.process.on("error", error => this.fail(error));
    this.process.on("exit", (code, signal) => {
      // stdout may still contain the last response when exit fires. Drain it
      // before failing pending requests, but do not enqueue new work meanwhile.
      this.exited = true;
      this.exitError = new Error(`Rust host exited (${signal ?? code})`);
    });
    this.process.on("close", () => this.fail(this.exitError ?? new Error("Rust host closed")));
    this.lines.on("close", () => this.fail(this.exitError ?? new Error("Rust host closed its output")));
    this.process.stdin.on("error", error => this.fail(error));
    this.process.stdout.on("error", error => this.fail(error));
  }
  fail(error) {
    if (this.closed) return;
    this.closed = true; this.failure = error;
    for (const request of this.pending.values()) { clearTimeout(request.timer); request.reject(error); }
    this.pending.clear();
    this.lines.close(); this.process.stdin.destroy();
    if (this.process.exitCode === null && this.process.signalCode === null) {
      this.process.kill();
      // A wedged host must not keep watch mode alive after protocol failure.
      const force = setTimeout(() => this.process.kill("SIGKILL"), 1_000);
      force.unref(); this.process.once("exit", () => clearTimeout(force));
    }
  }
  request(op, body = {}, timeoutMs = 120_000) {
    if (this.closed || this.exited) return Promise.reject(this.failure ?? this.exitError ?? new Error("Rust host is closed"));
    if (this.closing) return Promise.reject(new Error("Rust host is closing"));
    const id = this.nextId++;
    let message;
    try { message = JSON.stringify({ ...body, version: 1, id, op }) + "\n"; }
    catch (error) { return Promise.reject(error); }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.fail(new Error(`Rust host timed out during ${op}`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      try { this.process.stdin.write(message, error => { if (error) this.fail(error); }); }
      catch (error) { this.fail(error); }
    });
  }
  close(timeoutMs = 2_000) {
    if (this.closePromise) return this.closePromise;
    if (this.closed || this.exited) return Promise.resolve();
    const shutdown = this.request("shutdown", {}, timeoutMs);
    this.closing = true;
    this.closePromise = shutdown.finally(() => this.fail(new Error("Rust host is closed")));
    return this.closePromise;
  }
}
