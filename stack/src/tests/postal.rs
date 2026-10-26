//! 住所 ↔ 郵便番号（ADR 0032）の組み込みと学習

use super::*;

const POSTAL_SAMPLE: &str = include_str!("../../../dict-format/tests/fixtures/postal.sample.tsv");
const READING_A: &str = "とうきょうとちよだくちよだ";
const READING_B: &str = "とうきょうとちよだくまるのうち";
const LEARN_DAY: u32 = 20_000;

fn postal_stack(tag: &str) -> (Stack, PathBuf) {
    let dir = temp_dir(tag);
    let tsv = dir.join("postal.tsv");
    std::fs::write(&tsv, POSTAL_SAMPLE).unwrap();
    let (d, g) = mock();
    let paths = StackPaths {
        user_dir: Some(dir.clone()),
        ..Default::default()
    };
    let opts = StackOptions {
        postal_tsv: Some(tsv),
        ..eval_opts(RankerSet::Full)
    };
    (
        assemble(StackInputs::new(d, g), &paths, &opts).unwrap(),
        dir,
    )
}

fn first_page(stack: &Stack, reading: &str) -> Vec<String> {
    let mut conv = stack.converter.convert(reading, &Context::new());
    conv.page(0)
        .unwrap()
        .candidates
        .iter()
        .map(|c| c.surface.clone())
        .collect()
}

#[test]
fn postal_candidates_come_second_and_before_number_forms() {
    let (stack, dir) = postal_stack("postal-order");
    let a = first_page(&stack, READING_A);
    let plain = a.iter().position(|s| s == "1000001").expect("plain");
    assert_eq!(a[plain + 1], "100-0001", "{a:?}");
    assert_eq!(plain, 1, "{a:?}");
    // 郵便番号の読み: 打ったままが 1 位、住所はその次（NumberRewriter の 1,000,001 より先）
    let b = first_page(&stack, "1000001");
    assert_eq!(b[0], "1000001", "{b:?}");
    assert_eq!(b[1], "東京都千代田区千代田", "{b:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// ADR 0032 §12: Rewriter の候補の学習は (読み, 書式) で効く。ある住所で郵便番号の形を選んでも、
/// 別の住所や郵便番号の読みの候補の並びは変わらない。
#[test]
fn learning_a_postal_form_does_not_leak_to_other_readings() {
    let (stack, dir) = postal_stack("postal-learning");
    let before_b = first_page(&stack, READING_B);
    let before_code = first_page(&stack, "1000001");

    let mut conv = stack.converter.convert(READING_A, &Context::new());
    let hyphen = conv
        .page(0)
        .unwrap()
        .candidates
        .iter()
        .position(|c| c.surface == "100-0001")
        .unwrap();
    for _ in 0..3 {
        let event = conv.commit(hyphen).expect("commit event");
        stack
            .learning
            .as_ref()
            .unwrap()
            .record(&event, None, LEARN_DAY);
    }

    assert_eq!(first_page(&stack, READING_B), before_b);
    assert_eq!(first_page(&stack, "1000001"), before_code);
    // 選んだ読み自身では効く（100-0001 が 1000001 の前に出る）
    let a = first_page(&stack, READING_A);
    let h = a.iter().position(|s| s == "100-0001").unwrap();
    let p = a.iter().position(|s| s == "1000001").unwrap();
    assert!(h < p, "{a:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reuse_keeps_the_loaded_table_unless_it_is_dropped() {
    let (first, dir) = postal_stack("postal-reuse");
    let (d, g) = mock();
    let paths = StackPaths {
        user_dir: Some(dir.clone()),
        ..Default::default()
    };
    let opts = StackOptions {
        postal_tsv: Some(dir.join("postal.tsv")),
        ..eval_opts(RankerSet::Full)
    };
    let same = assemble_with(
        StackInputs::new(d, g),
        &paths,
        &opts,
        &Reuse::from_stack(&first),
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        first.postal.as_ref().unwrap(),
        same.postal.as_ref().unwrap()
    ));
    let (d, g) = mock();
    let reloaded = assemble_with(
        StackInputs::new(d, g),
        &paths,
        &opts,
        &Reuse {
            postal: None,
            ..Reuse::from_stack(&first)
        },
    )
    .unwrap();
    assert!(!Arc::ptr_eq(
        first.postal.as_ref().unwrap(),
        reloaded.postal.as_ref().unwrap()
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_table_or_japanese_off_gives_nothing() {
    let (d, g) = mock();
    let opts = StackOptions {
        postal_tsv: Some(PathBuf::from("/nonexistent/postal.tsv")),
        ..eval_opts(RankerSet::Analyzer)
    };
    let stack = assemble(StackInputs::new(d, g), &StackPaths::default(), &opts).unwrap();
    assert!(!first_page(&stack, READING_A).contains(&"1000001".to_owned()));

    let dir = temp_dir("postal-off");
    std::fs::write(dir.join("postal.tsv"), POSTAL_SAMPLE).unwrap();
    let (d, g) = mock();
    let opts = StackOptions {
        postal_tsv: Some(dir.join("postal.tsv")),
        postal: PostalOptions {
            japanese: false,
            ..PostalOptions::default()
        },
        ..eval_opts(RankerSet::Analyzer)
    };
    let stack = assemble(StackInputs::new(d, g), &StackPaths::default(), &opts).unwrap();
    assert!(!first_page(&stack, READING_A).contains(&"1000001".to_owned()));
    let _ = std::fs::remove_dir_all(dir);
}
