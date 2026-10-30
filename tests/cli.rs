use clap::Parser;
use oc_harness::cli::Cli;

#[test]
fn image_flag_is_not_a_cli_option() {
    assert!(Cli::try_parse_from(["d3", "--image", "screenshot.png", "describe"]).is_err());
}

#[test]
fn skip_approval_is_an_alias_for_yes() {
    for flag in ["--skip-approval", "--auto-approve", "--yes", "-y"] {
        let cli = Cli::try_parse_from(["d3", flag, "prompt"]).unwrap();
        assert!(cli.yes, "{flag} should enable automatic approval");
        assert_eq!(cli.prompt, ["prompt"]);
    }
    assert!(!Cli::try_parse_from(["d3", "prompt"]).unwrap().yes);
}
