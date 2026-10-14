//! A form's life, in one place (decision 0025, section 2): a form is asked for what a step
//! lacks ([`Palette::ask`]), typed (`form`), sent and kept as typed ([`Palette::keep`]); a
//! refusal that asks for an argument opens it asking that ([`Palette::refused`]), and any other
//! refusal of the step it sent opens it again as typed, with why under its field
//! ([`Palette::reopen`]). It ends once its step is answered ([`Palette::taken`]). What a refusal
//! leaves on the form (`refused`, `file`, `reason`) the form holds itself.

use gs_protocol::{Catalog, Step};

use super::{Form, Palette, Then, dialog, form, missing};

/// The form that sent a step, as it was typed, until the engine answers the step.
#[derive(Clone, Debug)]
pub(super) struct Kept {
    step: Step,
    form: Form,
    /// The window has taken the step to run: the next pass that does not run it ends this.
    taken: bool,
}

impl Palette {
    /// Keeps `form`, which sent `step`, as it was typed, for a refusal of that step to open
    /// again. A question is answered once, and keeps nothing.
    pub(super) fn keep(&mut self, step: &Step, form: Option<Form>) {
        let form = form.filter(|f| !f.is_question());
        self.kept = form.map(|form| Kept {
            step: step.clone(),
            form,
            taken: false,
        });
    }

    /// The window takes this pass's steps to run, the engine answering each in that pass, or as
    /// its job ends while `running` is the step. A kept form lives until its step's answer: a
    /// refusal opens it again, and anything else ends it, so the same step sent later (by a key)
    /// and refused says why on the status line and never brings the old form back.
    pub fn taken(&mut self, running: Option<&Step>) {
        let ended = |k: &Kept| k.taken && running != Some(&k.step);
        self.kept = (self.kept.take())
            .filter(|k| !ended(k))
            .map(|k| Kept { taken: true, ..k });
    }

    /// Opens it asking for the required arguments `step` lacks, and the optional ones named in
    /// `also` it lacks; when all it lacks is the file a file action names and the window shows
    /// dialogs, asks the system's file dialog instead. An error if no command has that name, or
    /// while another dialog waits for its answer.
    pub fn ask(&mut self, step: Step, catalog: &Catalog, also: &[String]) -> Result<(), String> {
        let info = (catalog.commands.iter()).find(|c| c.name == step.command);
        let info = info.ok_or_else(|| format!("no command called {}", step.command))?;
        let mut names = missing(info, &step.args);
        for name in also {
            if step.args.get(name.as_str()).is_none() && !names.contains(name) {
                names.push(name.clone());
            }
        }
        let wants = dialog::wants(&step.command).filter(|w| names == [w.arg]);
        if let Some(wants) = wants.filter(|_| self.dialogs) {
            return self.show_dialog(step, wants);
        }
        // A path starts in its folder; a part's new name starts as its name, selected, to type
        // over (review 0009, R15).
        let part = step.args.get("part").and_then(|p| p.as_str());
        let named = part.or(self.scene.active.as_deref());
        let start = |name: &str| match name {
            "path" => self.folder(&step.command).unwrap_or_default(),
            "name" if step.command == "part.rename" => named.unwrap_or_default().into(),
            _ => String::new(),
        };
        let fields = (names.into_iter())
            .map(|name| {
                let text = start(&name);
                (name, text)
            })
            .collect();
        self.set(true, None);
        self.focus = true;
        self.form = Some(Form::new(step, info, fields));
        Ok(())
    }

    /// Opens it asking for `asked`, the argument an engine's refusal of `step` names: a boolean
    /// so asked is a question with its buttons (decision 0025, rule 13), naming the file it is
    /// about; `discard_changes` is the question before unsaved changes are lost ([`unsaved`]).
    /// The question says why itself, never the status line (review 0009, C3): the unsaved
    /// question, where to save, and a file's Replace or Save in their own words; the others (a
    /// taken key's Replace) say `why`, the refusal in a person's words, in place of their line.
    ///
    /// [`unsaved`]: super::unsaved
    pub fn refused(
        &mut self,
        step: Step,
        catalog: &Catalog,
        asked: String,
        why: &str,
    ) -> Result<(), String> {
        if asked == "discard_changes" {
            self.set(true, None);
            self.unsaved.asking = Some(Then::Run(step, Vec::new()));
            return Ok(());
        }
        let file =
            (step.args["path"].as_str().map(String::from)).or_else(|| self.scene.own.clone());
        let own = asked == "path" || form::own_line(&asked, file.as_deref()).is_some();
        self.ask(step, catalog, std::slice::from_ref(&asked))?;
        if let Some(form) = &mut self.form {
            form.refused = true;
            form.file = file;
            form.reason = (!own).then(|| (asked, why.to_string()));
        }
        Ok(())
    }

    /// Opens again, as it was typed, the form that sent `step`, which the engine refused:
    /// `why` shows under the field the refusal points `at` (`/path`), else under the first
    /// (review 0009, C4). False, opening nothing, when no form sent it, the form asks nothing
    /// typed, or the palette is open on something else; the status line says why then.
    pub fn reopen(&mut self, step: &Step, at: Option<&str>, why: &str) -> bool {
        let Some(Kept {
            step: sent,
            mut form,
            ..
        }) = self.kept.take()
        else {
            return false;
        };
        if self.open || sent != *step {
            return false;
        }
        let named = at.map(|p| p.trim_start_matches('/').replace('/', "."));
        let fields = &form.fields;
        let named = named.filter(|n| fields.iter().any(|(f, _)| f == n));
        let Some(field) = named.or_else(|| fields.first().map(|(f, _)| f.clone())) else {
            return false;
        };
        form.reason = Some((field, why.to_string()));
        self.set(true, None);
        self.focus = true;
        self.form = Some(form);
        true
    }
}
