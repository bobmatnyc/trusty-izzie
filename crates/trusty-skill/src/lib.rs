//! `trusty-skill` — foundational skill trait and script-backed adapter.
//!
//! # Overview
//!
//! Every skill exposes a set of LLM-callable tools via the [`Skill`] trait.
//! [`ScriptSkill`] provides a zero-Rust path: drop a `.md` file with YAML
//! frontmatter into a directory and call [`load_script_skills`] at startup.

mod script;
mod skill;

pub use script::{load_script_skills, ScriptSkill};
pub use skill::{Skill, SkillResult, SkillTool, SkillToolCall};
