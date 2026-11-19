import { Sentry, type Session, type Offer } from "zmodem.js";

export interface ZmodemView {
  phase: "upload" | "offer" | "transfer";
  name: string;
  bytes: number;
  total: number;
}

export interface ZmodemIo {
  output(bytes: Uint8Array): void;
  send(bytes: number[]): Promise<void>;
  allowed(): boolean;
  view(state: ZmodemView | null): void;
  begin(name: string): Promise<string>;
  write(id: string, bytes: number[]): Promise<void>;
  finish(id: string, commit: boolean): Promise<string | null>;
  saved(path: string): void;
  error(error: unknown): void;
}

/** Binary protocol data never enters xterm or the AI's terminal-output listeners. */
export class ZmodemTerminal {
  private sentry: Sentry;
  private session: Session | null = null;
  private offer: Offer | null = null;
  private wire = Promise.resolve();
  private epoch = 0;
  private jobs = 0;
  private ended = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private downloads = new Set<string>();

  constructor(private io: ZmodemIo) {
    this.sentry = this.makeSentry();
  }

  private makeSentry() {
    return new Sentry({
      to_terminal: (data) => {
        if (!this.busy) this.io.output(Uint8Array.from(data));
      },
      sender: (data) => {
        const epoch = this.epoch;
        this.wire = this.wire
          .then(async () => {
            if (epoch === this.epoch && this.io.allowed()) await this.io.send(data);
          })
          .catch((e: unknown) => {
            if (epoch === this.epoch) this.fail(e);
          });
      },
      on_retract: () => {},
      on_detect: (detection) => {
        if (!this.io.allowed()) {
          detection.deny();
          return;
        }
        this.session = detection.confirm();
        this.ended = false;
        this.arm();
        this.session.on("session_end", () => {
          this.ended = true;
          if (!this.jobs) this.complete();
        });
        if (this.session.type === "send") this.io.view({ phase: "upload", name: "", bytes: 0, total: 0 });
        else {
          this.session.on("offer", (offer) => {
            this.offer = offer;
            const { name, size } = offer.get_details();
            this.io.view({ phase: "offer", name, bytes: 0, total: size });
          });
          this.session.start();
        }
      },
    });
  }

  get busy() {
    return this.session !== null;
  }

  consume(data: Uint8Array) {
    try {
      if (this.busy) this.arm();
      this.sentry.consume(data);
    } catch (e) {
      this.fail(e);
    }
  }

  private arm() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.fail(new Error("ZMODEM timed out")), 120_000);
  }

  private complete() {
    clearTimeout(this.timer);
    this.session = null;
    this.offer = null;
    this.io.view(null);
  }

  cancel() {
    const session = this.session;
    this.epoch++;
    this.jobs = 0;
    this.complete();
    // Send abort outside the old queue; queued file payloads are now invalidated.
    if (session) {
      try {
        session.abort();
      } catch {
        /* already ended */
      }
    }
    for (const id of this.downloads) void this.io.finish(id, false).catch(() => {});
    this.downloads.clear();
    this.sentry = this.makeSentry();
  }

  private fail(error: unknown) {
    this.cancel();
    this.io.error(error);
  }

  skip() {
    const offer = this.offer;
    this.offer = null;
    offer?.skip();
  }

  async receive() {
    const offer = this.offer;
    if (!offer) return;
    this.offer = null;
    const epoch = this.epoch;
    const { name, size } = offer.get_details();
    this.jobs++;
    let id: string | undefined;
    let writes = Promise.resolve();
    let pending = 0;
    let bytes = 0;
    let failure: unknown;
    try {
      id = await this.io.begin(name);
      if (epoch !== this.epoch) {
        await this.io.finish(id, false);
        return;
      }
      this.downloads.add(id);
      const token = id;
      this.io.view({ phase: "transfer", name, bytes, total: size });
      let buffer: number[] = [];
      const flush = () => {
        if (!buffer.length) return;
        const chunk = buffer;
        buffer = [];
        pending += chunk.length;
        if (pending > 8 * 1024 * 1024) throw new Error("ZMODEM disk queue exceeded 8 MiB");
        writes = writes
          .then(async () => {
            if (epoch === this.epoch && !failure) await this.io.write(token, chunk);
            pending -= chunk.length;
          })
          .catch((e: unknown) => {
            failure = e;
            if (epoch === this.epoch) this.fail(e);
          });
      };
      await offer.accept({
        on_input: (data) => {
          if (epoch !== this.epoch) throw new Error("ZMODEM cancelled");
          bytes += data.length;
          for (const byte of data) {
            buffer.push(byte);
            if (buffer.length === 64 * 1024) {
              flush();
              this.io.view({ phase: "transfer", name, bytes, total: size });
            }
          }
        },
      });
      flush();
      await writes;
      if (failure) throw failure;
      if (epoch !== this.epoch) return;
      if (Number.isFinite(size) && size >= 0 && bytes !== size) throw new Error("ZMODEM file size mismatch");
      const path = await this.io.finish(id, true);
      if (path) this.io.saved(path);
    } catch (e) {
      if (epoch === this.epoch) this.fail(e);
    } finally {
      if (id) {
        this.downloads.delete(id);
        await this.io.finish(id, false).catch(() => {});
      }
      if (epoch === this.epoch) this.jobs--;
      if (this.ended && !this.jobs && epoch === this.epoch) this.complete();
    }
  }

  async upload(files: readonly File[]) {
    const session = this.session;
    if (!session || session.type !== "send") return;
    const epoch = this.epoch;
    this.jobs++;
    this.io.view({ phase: "transfer", name: files[0]?.name ?? "", bytes: 0, total: files[0]?.size ?? 0 });
    try {
      for (const file of files) {
        if (epoch !== this.epoch) return;
        const transfer = await session.send_offer({ name: file.name, size: file.size });
        if (!transfer || epoch !== this.epoch) continue;
        for (let offset = 0; offset < file.size; offset += 64 * 1024) {
          const data = new Uint8Array(await file.slice(offset, offset + 64 * 1024).arrayBuffer());
          if (epoch !== this.epoch) return;
          transfer.send(data);
          await this.wire;
          this.arm();
          this.io.view({ phase: "transfer", name: file.name, bytes: offset + data.length, total: file.size });
        }
        await transfer.end();
      }
      if (epoch === this.epoch) await session.close();
    } catch (e) {
      if (epoch === this.epoch) this.fail(e);
    } finally {
      if (epoch === this.epoch) this.jobs--;
      if (this.ended && !this.jobs && epoch === this.epoch) this.complete();
    }
  }
}
