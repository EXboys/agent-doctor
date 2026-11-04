import { t } from "../i18n";

export interface DoctorStartCard {
  title: string;
  hint: string;
  /** Dropped into the composer for the person to edit, not sent as-is. */
  prompt: string;
}

/**
 * Starter questions for a built-in doctor chat, matched to what the doctor
 * can actually do: read the installed agents' configs, locate a problem and
 * guide the repair flow (which backs up first). Replaces the generic
 * onboarding trio, whose "open a doctor chat" card is meaningless inside a
 * doctor chat.
 */
export function doctorStartCards(): DoctorStartCard[] {
  return [
    {
      title: t("chat.doctorStartCheckup"),
      hint: t("chat.doctorStartCheckupHint"),
      prompt: t("chat.doctorStartCheckupPrompt"),
    },
    {
      title: t("chat.doctorStartDiagnose"),
      hint: t("chat.doctorStartDiagnoseHint"),
      prompt: t("chat.doctorStartDiagnosePrompt"),
    },
    {
      title: t("chat.doctorStartExplain"),
      hint: t("chat.doctorStartExplainHint"),
      prompt: t("chat.doctorStartExplainPrompt"),
    },
  ];
}
