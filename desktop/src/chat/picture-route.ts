import type { ImageReading } from "./context";

/**
 * `text`: mostly readable words (error, log, settings page) — the words alone answer it.
 * `picture`: chart, layout, photo, or few words — the model has to see it.
 */
export type PictureKind = "text" | "picture";

/** What this turn carries for its pictures. */
export type PictureTurn = {
  /** Paths handed to the model as real pictures. */
  sendPaths: string[];
  sentNames: string[];
  /** Words read on this computer, for pictures sent as text (or as backup). */
  readings: ImageReading[];
  /** Pictures the model gets nothing from: cannot see them and no words were read. */
  unseenNames: string[];
};

export type PictureInput = { path: string; name: string; text: string };

const TEXT_MIN_CHARS = 300;
const TEXT_MIN_LINES = 6;
const TEXT_MIN_WORD_RATIO = 0.6;

const WORD_CHAR = /[\p{L}]/u;

export function classifyPicture(text: string): PictureKind {
  const compact = text.replace(/\s+/g, "");
  if (compact.length < TEXT_MIN_CHARS) return "picture";
  const lines = text.split("\n").filter((line) => line.trim()).length;
  if (lines < TEXT_MIN_LINES) return "picture";
  let words = 0;
  for (const ch of compact) if (WORD_CHAR.test(ch)) words += 1;
  // Charts and tables read as digits and symbols; prose and errors read as words.
  return words / compact.length >= TEXT_MIN_WORD_RATIO ? "text" : "picture";
}

function extOf(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
}

export function planPictures(
  pictures: PictureInput[],
  support: { seesImages: boolean; formats: string[] },
): PictureTurn {
  const turn: PictureTurn = { sendPaths: [], sentNames: [], readings: [], unseenNames: [] };
  const formats = new Set(support.formats.map((f) => f.toLowerCase()));
  for (const item of pictures) {
    const text = item.text.trim();
    const canSend = support.seesImages && formats.has(extOf(item.path));
    if (canSend && (!text || classifyPicture(text) === "picture")) {
      turn.sendPaths.push(item.path);
      turn.sentNames.push(item.name);
      if (text) turn.readings.push({ name: item.name, text });
    } else if (text) {
      turn.readings.push({ name: item.name, text });
    } else {
      turn.unseenNames.push(item.name);
    }
  }
  return turn;
}
