use std::collections::HashMap;

use serde_json::value::Value as Json;

use crate::block::BlockContext;
use crate::context::{merge_json, Context};
use crate::error::RenderError;
use crate::json::path::Path;
use crate::output::Output;
use crate::registry::Registry;
use crate::render::{Decorator, Evaluable, RenderContext, Renderable};

pub(crate) const PARTIAL_BLOCK: &str = "@partial-block";

pub fn expand_partial<'reg: 'rc, 'rc>(
    d: &Decorator<'reg>,
    r: &'reg Registry<'reg>,
    ctx: &'rc Context,
    rc: &mut RenderContext<'reg>,
    out: &mut dyn Output,
) -> Result<(), RenderError> {
    // try eval inline partials first
    if let Some(t) = d.template() {
        t.eval(r, ctx, rc)?;
    }

    let tname = d.name();
    if rc.is_current_template(tname) {
        return Err(RenderError::new("Cannot include self in >"));
    }

    // if tname == PARTIAL_BLOCK
    let partial = rc
        .get_partial(tname)
        .or_else(|| r.get_template(tname))
        .or_else(|| d.template());

    if let Some(t) = partial {
        // clone to avoid lifetime issue
        // FIXME refactor this to avoid
        let mut local_rc = rc.clone();
        let is_partial_block = tname == PARTIAL_BLOCK;

        if is_partial_block {
            local_rc.inc_partial_block_depth();
        }

        let mut block_created = false;
        let param = d.param(0, r, ctx, &mut local_rc)?;
        let hash = d.hash(r, ctx, &mut local_rc)?;

        if let Some(ref p) = param {
            if let Some(ref base_path) = p.context_path() {
                // path given, update base_path
                let mut block = BlockContext::new();
                *block.base_path_mut() = base_path.to_vec();
                block_created = true;
                local_rc.push_block(block);
            }
        } else if !hash.is_empty() {
            let mut block = BlockContext::new();
            // hash given, update base_value
            let hash_ctx = hash
                .iter()
                .map(|(k, v)| (*k, v.value()))
                .collect::<HashMap<&str, &Json>>();

            let merged_context = merge_json(
                local_rc.evaluate2(ctx, &Path::current())?.as_json(),
                &hash_ctx,
            );
            block.set_base_value(merged_context);
            block_created = true;
            local_rc.push_block(block);
        }

        // @partial-block
        if let Some(pb) = d.template() {
            local_rc.push_partial_block(pb);
        }

        let result = t.render(r, ctx, &mut local_rc, out);

        // cleanup
        if block_created {
            local_rc.pop_block();
        }

        if is_partial_block {
            local_rc.dec_partial_block_depth();
        }

        if d.template().is_some() {
            local_rc.pop_partial_block();
        }

        result
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::context::Context;
    use crate::error::RenderError;
    use crate::output::Output;
    use crate::registry::Registry;
    use crate::render::{Helper, RenderContext};

    #[test]
    fn test() {
        let mut handlebars = Registry::new();
        assert!(handlebars
            .register_template_string("t0", "{{> t1}}")
            .is_ok());
        assert!(handlebars
            .register_template_string("t1", "{{this}}")
            .is_ok());
        assert!(handlebars
            .register_template_string("t2", "{{#> t99}}not there{{/t99}}")
            .is_ok());
        assert!(handlebars
            .register_template_string("t3", "{{#*inline \"t31\"}}{{this}}{{/inline}}{{> t31}}")
            .is_ok());
        assert!(handlebars
            .register_template_string(
                "t4",
                "{{#> t5}}{{#*inline \"nav\"}}navbar{{/inline}}{{/t5}}"
            )
            .is_ok());
        assert!(handlebars
            .register_template_string("t5", "include {{> nav}}")
            .is_ok());
        assert!(handlebars
            .register_template_string("t6", "{{> t1 a}}")
            .is_ok());
        assert!(handlebars
            .register_template_string(
                "t7",
                "{{#*inline \"t71\"}}{{a}}{{/inline}}{{> t71 a=\"world\"}}"
            )
            .is_ok());
        assert!(handlebars.register_template_string("t8", "{{a}}").is_ok());
        assert!(handlebars
            .register_template_string("t9", "{{> t8 a=2}}")
            .is_ok());

        assert_eq!(handlebars.render("t0", &1).ok().unwrap(), "1".to_string());
        assert_eq!(
            handlebars.render("t2", &1).ok().unwrap(),
            "not there".to_string()
        );
        assert_eq!(handlebars.render("t3", &1).ok().unwrap(), "1".to_string());
        assert_eq!(
            handlebars.render("t4", &1).ok().unwrap(),
            "include navbar".to_string()
        );
        assert_eq!(
            handlebars
                .render("t6", &btreemap! {"a".to_string() => "2".to_string()})
                .ok()
                .unwrap(),
            "2".to_string()
        );
        assert_eq!(
            handlebars.render("t7", &1).ok().unwrap(),
            "world".to_string()
        );
        assert_eq!(handlebars.render("t9", &1).ok().unwrap(), "2".to_string());
    }

    #[test]
    fn test_include_partial_block() {
        let t0 = "hello {{> @partial-block}}";
        let t1 = "{{#> t0}}inner {{this}}{{/t0}}";

        let mut handlebars = Registry::new();
        assert!(handlebars.register_template_string("t0", t0).is_ok());
        assert!(handlebars.register_template_string("t1", t1).is_ok());

        let r0 = handlebars.render("t1", &true);
        assert_eq!(r0.ok().unwrap(), "hello inner true".to_string());
    }

    #[test]
    fn test_self_inclusion() {
        let t0 = "hello {{> t1}} {{> t0}}";
        let t1 = "some template";
        let mut handlebars = Registry::new();
        assert!(handlebars.register_template_string("t0", t0).is_ok());
        assert!(handlebars.register_template_string("t1", t1).is_ok());

        let r0 = handlebars.render("t0", &true);
        assert!(r0.is_err());
    }

    #[test]
    fn test_issue_143() {
        let main_template = "one{{> two }}three{{> two }}";
        let two_partial = "--- two ---";

        let mut handlebars = Registry::new();
        assert!(handlebars
            .register_template_string("template", main_template)
            .is_ok());
        assert!(handlebars
            .register_template_string("two", two_partial)
            .is_ok());

        let r0 = handlebars.render("template", &true);
        assert_eq!(r0.ok().unwrap(), "one--- two ---three--- two ---");
    }

    #[test]
    fn test_hash_context_outscope() {
        let main_template = "In: {{> p a=2}} Out: {{a}}";
        let p_partial = "{{a}}";

        let mut handlebars = Registry::new();
        assert!(handlebars
            .register_template_string("template", main_template)
            .is_ok());
        assert!(handlebars.register_template_string("p", p_partial).is_ok());

        let r0 = handlebars.render("template", &true);
        assert_eq!(r0.ok().unwrap(), "In: 2 Out: ");
    }

    #[test]
    fn test_partial_context_hash() {
        let mut hbs = Registry::new();
        hbs.register_template_string("one", "This is a test. {{> two name=\"fred\" }}")
            .unwrap();
        hbs.register_template_string("two", "Lets test {{name}}")
            .unwrap();
        assert_eq!(
            "This is a test. Lets test fred",
            hbs.render("one", &0).unwrap()
        );
    }

    #[test]
    fn test_partial_subexpression_context_hash() {
        let mut hbs = Registry::new();
        hbs.register_template_string("one", "This is a test. {{> (x @root) name=\"fred\" }}")
            .unwrap();
        hbs.register_template_string("two", "Lets test {{name}}")
            .unwrap();

        hbs.register_helper(
            "x",
            Box::new(
                |_: &Helper<'_>,
                 _: &Registry<'_>,
                 _: &Context,
                 _: &mut RenderContext<'_>,
                 out: &mut dyn Output|
                 -> Result<(), RenderError> {
                    out.write("two")?;
                    Ok(())
                },
            ),
        );
        assert_eq!(
            "This is a test. Lets test fred",
            hbs.render("one", &0).unwrap()
        );
    }

    #[test]
    fn test_nested_partial_scope() {
        let t = "{{#*inline \"pp\"}}{{a}} {{b}}{{/inline}}{{#each c}}{{> pp a=2}}{{/each}}";
        let data = json!({"c": [{"b": true}, {"b": false}]});

        let mut handlebars = Registry::new();
        assert!(handlebars.register_template_string("t", t).is_ok());
        let r0 = handlebars.render("t", &data);
        assert_eq!(r0.ok().unwrap(), "2 true2 false");
    }

    #[test]
    fn test_nested_partials() {
        let mut handlebars = Registry::new();
        let template1 = "<outer>{{> @partial-block }}</outer>";
        let template2 = "{{#> t1 }}<inner>{{> @partial-block }}</inner>{{/ t1 }}";
        let template3 = "{{#> t2 }}Hello{{/ t2 }}";

        handlebars
            .register_template_string("t1", &template1)
            .unwrap();
        handlebars
            .register_template_string("t2", &template2)
            .unwrap();

        let page = handlebars.render_template(&template3, &json!({})).unwrap();
        assert_eq!("<outer><inner>Hello</inner></outer>", page);
    }
}
