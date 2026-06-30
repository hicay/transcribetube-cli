import { ProjectWithTranscript, TranscriptParagraph } from '../api/client';

export type TranscriptFormat = 'txt' | 'srt' | 'vtt' | 'md' | 'json';

export interface FormatOptions {
  speakers?: boolean;
  timecodes?: boolean;
}

/**
 * Render a stored transcript ({ transcription[], speakers{} }) into the
 * requested output format. These converters live client-side so callers
 * never have to parse the raw Deepgram-derived shape themselves.
 */
export function formatTranscript(
  project: ProjectWithTranscript,
  format: TranscriptFormat,
  opts: FormatOptions = {},
): string {
  const paragraphs = project.transcription ?? [];
  const names = project.speakers ?? {};
  const o = { speakers: true, timecodes: true, ...opts };

  switch (format) {
    case 'json':
      return JSON.stringify(
        { transcription: paragraphs, speakers: names },
        null,
        2,
      );
    case 'srt':
      return toSrt(paragraphs, names, o);
    case 'vtt':
      return toVtt(paragraphs, names, o);
    case 'md':
      return toMarkdown(project, paragraphs, names);
    case 'txt':
    default:
      return toText(paragraphs, names, o);
  }
}

function speakerName(
  p: TranscriptParagraph,
  names: Record<string, string>,
): string | undefined {
  if (p.speaker === undefined || p.speaker === null) return undefined;
  const key = String(p.speaker);
  return names[key] || `Speaker ${key}`;
}

function toText(
  paragraphs: TranscriptParagraph[],
  names: Record<string, string>,
  o: Required<FormatOptions>,
): string {
  return paragraphs
    .map((p) => {
      const parts: string[] = [];
      if (o.timecodes) parts.push(`[${clock(p.start)}]`);
      const name = o.speakers ? speakerName(p, names) : undefined;
      if (name) parts.push(`${name}:`);
      parts.push(p.text.trim());
      return parts.join(' ');
    })
    .join('\n');
}

function toMarkdown(
  project: ProjectWithTranscript,
  paragraphs: TranscriptParagraph[],
  names: Record<string, string>,
): string {
  const lines = [`# ${project.name || 'Transcript'}`, ''];
  let lastSpeaker: string | undefined;
  for (const p of paragraphs) {
    const name = speakerName(p, names);
    if (name && name !== lastSpeaker) {
      lines.push('', `**${name}**`);
      lastSpeaker = name;
    }
    lines.push(`${p.text.trim()} \`${clock(p.start)}\``);
  }
  return lines.join('\n');
}

function toSrt(
  paragraphs: TranscriptParagraph[],
  names: Record<string, string>,
  o: Required<FormatOptions>,
): string {
  return paragraphs
    .map((p, i) => {
      const name = o.speakers ? speakerName(p, names) : undefined;
      const text = name ? `${name}: ${p.text.trim()}` : p.text.trim();
      return `${i + 1}\n${stamp(p.start, ',')} --> ${stamp(p.end, ',')}\n${text}\n`;
    })
    .join('\n');
}

function toVtt(
  paragraphs: TranscriptParagraph[],
  names: Record<string, string>,
  o: Required<FormatOptions>,
): string {
  const cues = paragraphs
    .map((p) => {
      const name = o.speakers ? speakerName(p, names) : undefined;
      const text = name ? `<v ${name}>${p.text.trim()}` : p.text.trim();
      return `${stamp(p.start, '.')} --> ${stamp(p.end, '.')}\n${text}\n`;
    })
    .join('\n');
  return `WEBVTT\n\n${cues}`;
}

/** Seconds -> HH:MM:SS,mmm (or with '.' separator for VTT). */
function stamp(seconds: number, sep: ',' | '.'): string {
  const ms = Math.floor((seconds % 1) * 1000);
  return `${clock(seconds)}${sep}${pad(ms, 3)}`;
}

/** Seconds -> HH:MM:SS. */
function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return `${pad(h)}:${pad(m)}:${pad(s % 60)}`;
}

function pad(n: number, width = 2): string {
  return String(n).padStart(width, '0');
}
