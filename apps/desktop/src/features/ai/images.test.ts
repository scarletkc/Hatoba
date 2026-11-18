import { describe, expect, it } from "vitest";
import {
  attachmentBytes,
  composeMessage,
  fitsMessage,
  parseMessage,
  withinAttachmentLimits,
  type ImageAttachment,
} from "./attachments";
import { IMAGE_MAX_BYTES, IMAGE_TOKENS, imageBase64, imageMime, imageUrl } from "./images";
import { estimateTokens } from "./meter";
import { withinCap } from "./queue";

const image: ImageAttachment = { kind: "image", name: 'screen "1".png', mime: "image/png", text: "aGk=" };

describe("image attachments", () => {
  it("round-trips images, names, text attachments and questions", () => {
    const text = composeMessage("What does this error mean?", [
      image,
      { kind: "file", name: "a.txt", lines: 1, text: "error" },
      image,
    ]);
    const parsed = parseMessage(text);
    expect(parsed.attachments).toEqual([image, { kind: "file", name: "a.txt", lines: 1, text: "error" }, image]);
    expect(parsed.typed).toBe("What does this error mean?");
  });

  it("sniffs supported formats instead of trusting file extensions", () => {
    expect(imageMime(new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]))).toBe("image/png");
    expect(imageMime(new Uint8Array([255, 216, 255]))).toBe("image/jpeg");
    expect(imageMime(new TextEncoder().encode("GIF89a"))).toBe("image/gif");
    expect(imageMime(new TextEncoder().encode("RIFF1234WEBP"))).toBe("image/webp");
    for (const data of ["<svg/>", "not an image", "RIFF1234WAVE", ""])
      expect(imageMime(new TextEncoder().encode(data))).toBeNull();
  });

  it("encodes large buffers without overflowing the call stack", () => {
    const bytes = new Uint8Array(100_000).fill(137);
    expect(atob(imageBase64(bytes))).toHaveLength(bytes.length);
    expect(imageUrl(image)).toBe("data:image/png;base64,aGk=");
    expect(attachmentBytes(image)).toBe(2);
  });

  it("keeps separate limits for images and text, including queued messages", () => {
    const large = { ...image, text: imageBase64(new Uint8Array(IMAGE_MAX_BYTES)) };
    expect(fitsMessage([], large)).toBe(true);
    expect(fitsMessage([large], large)).toBe(true);
    expect(fitsMessage([large, large], image)).toBe(false);
    expect(fitsMessage([], { ...large, text: imageBase64(new Uint8Array(IMAGE_MAX_BYTES + 1)) })).toBe(false);
    expect(withinAttachmentLimits(Array.from({ length: 5 }, () => image))).toBe(false);
    expect(fitsMessage([large], { kind: "paste", lines: 1, text: "x".repeat(512 * 1024) })).toBe(true);
    expect(fitsMessage([image], { kind: "paste", lines: 1, text: "x".repeat(512 * 1024 + 1) })).toBe(false);
    expect(
      withinCap({ typed: "why?", chips: [], extras: [{ id: "image", attachment: large }], restore: () => [] }),
    ).toBe(true);
  });

  it("recognizes images after a shell change note", () => {
    const prefix = '<shell_change from="sh" to="bash">\nShell changed\n</shell_change>\n\n';
    const text = prefix + composeMessage("why?", [image]);
    expect(parseMessage(text).attachments).toEqual([image]);
    expect(estimateTokens(text)).toBe(IMAGE_TOKENS + Math.ceil((prefix.length + 4) / 4));
  });

  it("estimates image tokens rather than counting base64 as text", () => {
    const text = composeMessage("why?", [{ ...image, text: "A".repeat(1024 * 1024) }]);
    expect(estimateTokens(text)).toBe(IMAGE_TOKENS + 1);
    expect(estimateTokens(composeMessage("why?", [image, image]))).toBe(IMAGE_TOKENS * 2 + 1);
    const example = `Example:\n\n${text}`;
    expect(estimateTokens(example)).toBe(Math.ceil(example.length / 4));
    const file = composeMessage("why?", [{ kind: "file", name: "example.txt", lines: 5, text }]);
    expect(estimateTokens(file)).toBe(Math.ceil(file.length / 4));
  });
});
