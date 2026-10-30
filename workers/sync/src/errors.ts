import type { ContentfulStatusCode } from "hono/utils/http-status";

/** An error that is rendered to the client as `{ error, message? }` with the given status. */
export class ApiError extends Error {
  readonly status: ContentfulStatusCode;
  readonly code: string;
  readonly headers: Record<string, string> | undefined;

  constructor(
    status: ContentfulStatusCode,
    code: string,
    message?: string,
    headers?: Record<string, string>,
  ) {
    super(message ?? code);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.headers = headers;
  }

  /** Whether a human-readable message was supplied (otherwise `message` just repeats the code). */
  get hasMessage(): boolean {
    return this.message !== this.code;
  }
}

export const notInitialized = () =>
  new ApiError(404, "not_initialized", "The vault has not been set up on this Worker yet");

export const invalidCredentials = () => new ApiError(401, "invalid_credentials");

export const invalidSession = () =>
  new ApiError(401, "invalid_session", "Session is unknown or has expired", { "WWW-Authenticate": "Bearer" });
