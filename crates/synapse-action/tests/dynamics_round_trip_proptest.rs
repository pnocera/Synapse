use proptest::{
    collection::vec,
    prelude::*,
    test_runner::{Config, TestCaseError, TestRng, TestRunner},
};
use synapse_action::{
    ActionBackend, EmitState, RecordedInput, RecordingBackend, sample_typing_schedule,
};
use synapse_core::{Action, Backend, KeystrokeDynamics, KeystrokeNaturalParams};

#[test]
fn empty_string_records_zero_events_fsv() -> Result<(), Box<dyn std::error::Error>> {
    let text = "";
    let (schedule_chars, events, reconstructed) = record_and_reconstruct(text)?;
    println!(
        "source_of_truth=dynamics_round_trip edge=empty before={text:?} after=events:{events:?},reconstructed:{reconstructed:?} final_value=events:{}",
        events.len()
    );

    assert!(schedule_chars.is_empty());
    assert!(events.is_empty());
    assert_eq!(reconstructed, text);
    Ok(())
}

#[test]
fn extended_latin_string_round_trips_fsv() -> Result<(), Box<dyn std::error::Error>> {
    let text = "Az ÀĿſƀ";
    let (schedule_chars, events, reconstructed) = record_and_reconstruct(text)?;
    println!(
        "source_of_truth=dynamics_round_trip edge=extended_latin before={text:?} after=schedule:{schedule_chars:?},events:{events:?},reconstructed:{reconstructed:?} final_value={reconstructed:?}"
    );

    assert_eq!(schedule_chars, text);
    assert_eq!(reconstructed, text);
    assert_eq!(unicode_down_count(&events), text.encode_utf16().count());
    Ok(())
}

#[test]
fn random_strings_round_trip_through_recording_backend_10k()
-> Result<(), Box<dyn std::error::Error>> {
    let config = Config {
        cases: 10_000,
        failure_persistence: None,
        ..Config::default()
    };
    let algorithm = config.rng_algorithm;
    let mut runner = TestRunner::new_with_rng(config, TestRng::deterministic_rng(algorithm));

    runner.run(&text_strategy(), |text| {
        let (schedule_chars, events, reconstructed) = record_and_reconstruct(&text)
            .map_err(|error| TestCaseError::fail(format!("input={text:?} error={error}")))?;
        let expected_units = text.encode_utf16().count();

        prop_assert_eq!(
            &schedule_chars,
            &text,
            "input={:?} schedule={:?} events={:?}",
            text,
            schedule_chars,
            events
        );
        prop_assert_eq!(
            unicode_down_count(&events),
            expected_units,
            "input={:?} events={:?}",
            text,
            events
        );
        prop_assert_eq!(
            events.len(),
            expected_units.saturating_mul(2),
            "input={:?} events={:?}",
            text,
            events
        );
        prop_assert_eq!(
            &reconstructed,
            &text,
            "input={:?} events={:?}",
            text,
            events
        );
        Ok(())
    })?;

    println!(
        "source_of_truth=dynamics_round_trip edge=proptest final_value=ok cases=10000 max_chars=200 unicode_range=U+00C0..U+017F"
    );
    Ok(())
}

fn text_strategy() -> impl Strategy<Value = String> {
    vec(
        prop_oneof![
            prop::char::range('\u{0000}', '\u{007f}'),
            prop::char::range('\u{00c0}', '\u{017f}'),
        ],
        0..=200,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

fn record_and_reconstruct(
    text: &str,
) -> Result<(String, Vec<RecordedInput>, String), Box<dyn std::error::Error>> {
    let dynamics = KeystrokeDynamics::Natural {
        params: KeystrokeNaturalParams::FAST,
    };
    let schedule = sample_typing_schedule(text, &dynamics, Some(42));
    let schedule_chars: String = schedule.iter().map(|event| event.r#char).collect();
    let backend = RecordingBackend::new();
    let mut state = EmitState::new();
    backend.execute(
        &Action::TypeText {
            text: text.to_owned(),
            dynamics,
            backend: Backend::Software,
        },
        &mut state,
    )?;
    let events = backend.events();
    let reconstructed = reconstruct_from_unicode_down_events(&events)?;
    Ok((schedule_chars, events, reconstructed))
}

fn reconstruct_from_unicode_down_events(
    events: &[RecordedInput],
) -> Result<String, Box<dyn std::error::Error>> {
    let units: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            RecordedInput::UnicodeUnitDown { unit } => Some(*unit),
            RecordedInput::KeyDown { .. }
            | RecordedInput::KeyUp { .. }
            | RecordedInput::DelayMs { .. }
            | RecordedInput::UnicodeUnitUp { .. }
            | RecordedInput::MouseMove { .. }
            | RecordedInput::MouseMoveAbsolute { .. }
            | RecordedInput::MouseMoveRelative { .. }
            | RecordedInput::MouseButtonDown { .. }
            | RecordedInput::MouseButtonUp { .. }
            | RecordedInput::MouseScroll { .. }
            | RecordedInput::AimAt { .. }
            | RecordedInput::ComboAt { .. }
            | RecordedInput::PadButtonDown { .. }
            | RecordedInput::PadButtonUp { .. }
            | RecordedInput::PadStick { .. }
            | RecordedInput::PadTrigger { .. }
            | RecordedInput::PadReport { .. }
            | RecordedInput::ReleaseAll { .. } => None,
        })
        .collect();
    Ok(String::from_utf16(&units)?)
}

fn unicode_down_count(events: &[RecordedInput]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, RecordedInput::UnicodeUnitDown { .. }))
        .count()
}
