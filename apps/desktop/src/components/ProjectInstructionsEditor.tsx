import { useEffect, useRef, useState, type FormEvent } from "react";
import type { Project } from "../platform/projects";

type SavePhase = "idle" | "saving" | "failed";
type LineEnding = "\n" | "\r" | "\r\n";

interface TextareaEditSnapshot {
  instructions: string;
  selectionEnd: number;
  selectionStart: number;
}

interface TextareaSelection {
  end: number;
  start: number;
}

const lineEndingPattern = /\r\n|\r|\n/g;

function getLineEndings(instructions: string): LineEnding[] {
  return Array.from(instructions.matchAll(lineEndingPattern), (match) => match[0] as LineEnding);
}

function getPreferredLineEnding(instructions: string, fallback: LineEnding = "\n"): LineEnding {
  const lineEndings = getLineEndings(instructions);

  if (lineEndings.length === 0) {
    return fallback;
  }

  const counts = new Map<LineEnding, number>();

  for (const lineEnding of lineEndings) {
    counts.set(lineEnding, (counts.get(lineEnding) ?? 0) + 1);
  }

  return lineEndings.reduce((preferred, candidate) =>
    (counts.get(candidate) ?? 0) > (counts.get(preferred) ?? 0) ? candidate : preferred,
  );
}

function toTextareaValue(instructions: string): string {
  return instructions.replace(/\r\n|\r/g, "\n");
}

function getLineBreakPositions(instructions: string): number[] {
  const positions: number[] = [];

  for (let index = 0; index < instructions.length; index += 1) {
    if (instructions[index] === "\n") {
      positions.push(index);
    }
  }

  return positions;
}

function applyLineEndings(instructions: string, lineEndings: readonly LineEnding[]): string {
  let result = "";
  let contentStart = 0;
  let lineEndingIndex = 0;

  for (const lineBreakPosition of getLineBreakPositions(instructions)) {
    result += instructions.slice(contentStart, lineBreakPosition);
    result += lineEndings[lineEndingIndex];
    contentStart = lineBreakPosition + 1;
    lineEndingIndex += 1;
  }

  return result + instructions.slice(contentStart);
}

function getCommonPrefixLength(first: string, second: string, maximumLength?: number): number {
  const limit = Math.min(first.length, second.length, maximumLength ?? Number.POSITIVE_INFINITY);
  let length = 0;

  while (length < limit && first[length] === second[length]) {
    length += 1;
  }

  return length;
}

function getCommonSuffixLength(
  first: string,
  second: string,
  prefixLength: number,
  maximumLength?: number,
): number {
  const limit = Math.min(
    first.length - prefixLength,
    second.length - prefixLength,
    maximumLength ?? Number.POSITIVE_INFINITY,
  );
  let length = 0;

  while (
    length < limit &&
    first[first.length - length - 1] === second[second.length - length - 1]
  ) {
    length += 1;
  }

  return length;
}

function restoreLineEndings(
  previousInstructions: string,
  nextTextareaValue: string,
  preferredLineEnding: LineEnding,
  previousSelection?: TextareaSelection,
  nextSelection?: TextareaSelection,
): string {
  const previousTextareaValue = toTextareaValue(previousInstructions);
  const normalizedNextValue = toTextareaValue(nextTextareaValue);
  const previousLineEndings = getLineEndings(previousInstructions);
  const previousLineBreakPositions = getLineBreakPositions(previousTextareaValue);
  const nextLineBreakPositions = getLineBreakPositions(normalizedNextValue);

  const hasEditSelection = previousSelection !== undefined && nextSelection !== undefined;
  const commonPrefixLength = getCommonPrefixLength(
    previousTextareaValue,
    normalizedNextValue,
    hasEditSelection
      ? Math.min(previousSelection.start, nextSelection.start)
      : Number.POSITIVE_INFINITY,
  );
  const maximumSuffixLength = hasEditSelection
    ? Math.min(
        previousTextareaValue.length - previousSelection.end,
        normalizedNextValue.length - nextSelection.end,
      )
    : Number.POSITIVE_INFINITY;
  const commonSuffixLength = getCommonSuffixLength(
    previousTextareaValue,
    normalizedNextValue,
    commonPrefixLength,
    maximumSuffixLength,
  );
  const previousLineEndingsByPosition = new Map(
    previousLineBreakPositions.map((position, index) => [position, previousLineEndings[index]]),
  );
  const lengthDifference = previousTextareaValue.length - normalizedNextValue.length;
  const nextSuffixStart = normalizedNextValue.length - commonSuffixLength;
  const nextLineEndings = nextLineBreakPositions.map((position) => {
    let previousPosition: number | null = null;

    if (position < commonPrefixLength) {
      previousPosition = position;
    } else if (position >= nextSuffixStart) {
      previousPosition = position + lengthDifference;
    }

    return (
      (previousPosition === null
        ? undefined
        : previousLineEndingsByPosition.get(previousPosition)) ?? preferredLineEnding
    );
  });

  return applyLineEndings(normalizedNextValue, nextLineEndings);
}

interface ProjectInstructionsEditorProps {
  project: Project;
  onDirtyChange: (projectId: string, hasUnsavedChanges: boolean) => void;
  onSave: (projectId: string, instructions: string) => Promise<Project>;
}

interface ProjectInstructionsEditorSessionProps extends ProjectInstructionsEditorProps {
  initialInstructions: string;
}

function ProjectInstructionsEditorSession({
  initialInstructions,
  project,
  onDirtyChange,
  onSave,
}: ProjectInstructionsEditorSessionProps) {
  const [draft, setDraft] = useState(initialInstructions);
  const [savedInstructions, setSavedInstructions] = useState(initialInstructions);
  const [preferredLineEnding, setPreferredLineEnding] = useState<LineEnding>(() =>
    getPreferredLineEnding(initialInstructions),
  );
  const [savePhase, setSavePhase] = useState<SavePhase>("idle");
  const pendingEdit = useRef<TextareaEditSnapshot | null>(null);
  const hasUnsavedChanges = draft !== savedInstructions;

  useEffect(() => {
    onDirtyChange(project.id, hasUnsavedChanges);
  }, [hasUnsavedChanges, onDirtyChange, project.id]);

  let statusMessage = "Saved";
  let statusClassName = "instructions-save-state instructions-save-state-saved";

  if (savePhase === "saving") {
    statusMessage = "Saving…";
    statusClassName = "instructions-save-state instructions-save-state-saving";
  } else if (savePhase === "failed") {
    statusMessage = "Save failed. Your changes are still here.";
    statusClassName = "instructions-save-state instructions-save-state-failed";
  } else if (hasUnsavedChanges) {
    statusMessage = "Unsaved changes";
    statusClassName = "instructions-save-state instructions-save-state-unsaved";
  }

  function captureEditSelection(textarea: HTMLTextAreaElement) {
    pendingEdit.current = {
      instructions: draft,
      selectionEnd: textarea.selectionEnd,
      selectionStart: textarea.selectionStart,
    };
  }

  function updateDraft(textarea: HTMLTextAreaElement) {
    const editSnapshot = pendingEdit.current;
    const nextSelection = {
      end: textarea.selectionEnd,
      start: textarea.selectionStart,
    };
    const textareaValue = textarea.value;
    pendingEdit.current = null;

    setDraft((currentDraft) => {
      const previousSelection =
        editSnapshot?.instructions === currentDraft
          ? {
              end: editSnapshot.selectionEnd,
              start: editSnapshot.selectionStart,
            }
          : undefined;

      return restoreLineEndings(
        currentDraft,
        textareaValue,
        preferredLineEnding,
        previousSelection,
        previousSelection === undefined ? undefined : nextSelection,
      );
    });
    setSavePhase("idle");
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();

    if (!hasUnsavedChanges || savePhase === "saving") {
      return;
    }

    const instructions = draft;
    setSavePhase("saving");

    try {
      const savedProject = await onSave(project.id, instructions);
      setDraft(savedProject.instructions);
      setSavedInstructions(savedProject.instructions);
      setPreferredLineEnding((currentLineEnding) =>
        getPreferredLineEnding(savedProject.instructions, currentLineEnding),
      );
      setSavePhase("idle");
    } catch {
      setSavePhase("failed");
    }
  }

  return (
    <form className="project-instructions-editor" onSubmit={(event) => void handleSubmit(event)}>
      <div className="instructions-heading">
        <h3 id="project-instructions-title">Instructions</h3>
        <span
          className={statusClassName}
          id="project-instructions-status"
          role={savePhase === "failed" ? "alert" : "status"}
        >
          {statusMessage}
        </span>
      </div>

      <label className="sr-only" htmlFor="project-instructions">
        Project instructions
      </label>
      <textarea
        id="project-instructions"
        name="instructions"
        value={toTextareaValue(draft)}
        disabled={savePhase === "saving"}
        aria-describedby="project-instructions-status"
        onBeforeInput={(event) => captureEditSelection(event.currentTarget)}
        onChange={(event) => updateDraft(event.currentTarget)}
        onKeyDown={(event) => captureEditSelection(event.currentTarget)}
      />
      <div className="instructions-actions">
        <button
          className="primary-action"
          type="submit"
          disabled={!hasUnsavedChanges || savePhase === "saving"}
        >
          {savePhase === "saving" ? "Saving…" : "Save instructions"}
        </button>
      </div>
    </form>
  );
}

export function ProjectInstructionsEditor({
  project,
  onDirtyChange,
  onSave,
}: ProjectInstructionsEditorProps) {
  return (
    <ProjectInstructionsEditorSession
      key={`${project.id}\u0000${project.instructions}`}
      initialInstructions={project.instructions}
      project={project}
      onDirtyChange={onDirtyChange}
      onSave={onSave}
    />
  );
}
