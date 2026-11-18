declare module "zmodem.js" {
  export interface Offer {
    get_details(): { name: string; size: number };
    accept(options: { on_input: (data: number[]) => void }): Promise<void>;
    skip(): void;
  }
  export interface Transfer {
    send(data: Uint8Array): void;
    end(data?: Uint8Array): Promise<void>;
  }
  export interface Session {
    type: "send" | "receive";
    on(event: "offer", handler: (offer: Offer) => void): void;
    on(event: "session_end", handler: () => void): void;
    start(): void;
    abort(): void;
    has_ended(): boolean;
    send_offer(details: { name: string; size: number }): Promise<Transfer | undefined>;
    close(): Promise<void>;
  }
  export interface Detection {
    confirm(): Session;
    deny(): void;
  }
  export class Sentry {
    constructor(options: {
      to_terminal(data: number[]): void;
      sender(data: number[]): void;
      on_detect(detection: Detection): void;
      on_retract(): void;
    });
    consume(data: Uint8Array): void;
  }
}
