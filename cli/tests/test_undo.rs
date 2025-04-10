// Copyright 2022 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_undo_rewrite_with_child() {
    // Test that if we undo an operation that rewrote some commit, any descendants
    // after that will be rebased on top of the un-rewritten commit.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "initial"]).success();
    work_dir.run_jj(["describe", "-m", "modified"]).success();
    let output = work_dir.run_jj(["op", "log"]).success();
    let op_id_hex = output.stdout.raw()[3..15].to_string();
    work_dir.run_jj(["new", "-m", "child"]).success();
    let output = work_dir.run_jj(["log", "-T", "description"]);
    insta::assert_snapshot!(output, @r"
    @  child
    ○  modified
    ◆
    [EOF]
    ");
    work_dir.run_jj(["undo", &op_id_hex]).success();

    // Since we undid the description-change, the child commit should now be on top
    // of the initial commit
    let output = work_dir.run_jj(["log", "-T", "description"]);
    insta::assert_snapshot!(output, @r"
    @  child
    ○  initial
    ◆
    [EOF]
    ");
}

#[test]
fn test_git_push_undo() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(".", ["git", "clone", "git-repo", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["describe", "-m", "AA"]).success();
    work_dir.run_jj(["git", "push", "--allow-new"]).success();
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "BB"]).success();
    //   Refs at this point look as follows (-- means no ref)
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | AA
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    [EOF]
    ");
    let pre_push_opid = work_dir.current_operation_id();
    work_dir.run_jj(["git", "push"]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @origin: qpvuntsm 75e78001 (empty) BB
    [EOF]
    ");

    // Undo the push
    work_dir.run_jj(["op", "restore", &pre_push_opid]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "CC"]).success();
    work_dir.run_jj(["git", "fetch"]).success();
    // TODO: The user would probably not expect a conflict here. It currently is
    // because the undo made us forget that the remote was at v2, so the fetch
    // made us think it updated from v1 to v2 (instead of the no-op it could
    // have been).
    //
    // One option to solve this would be to have undo not restore remote-tracking
    // bookmarks, but that also has undersired consequences: the second fetch in
    // `jj git fetch && jj undo && jj git fetch` would become a no-op.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main (conflicted):
      - qpvuntsm hidden 2080bdb8 (empty) AA
      + qpvuntsm?? 20b2cc4b (empty) CC
      + qpvuntsm?? 75e78001 (empty) BB
      @origin (behind by 1 commits): qpvuntsm?? 75e78001 (empty) BB
    [EOF]
    ");
}

/// This test is identical to the previous one, except for one additional
/// import. It demonstrates that this changes the outcome.
#[test]
fn test_git_push_undo_with_import() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(".", ["git", "clone", "git-repo", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["describe", "-m", "AA"]).success();
    work_dir.run_jj(["git", "push", "--allow-new"]).success();
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "BB"]).success();
    //   Refs at this point look as follows (-- means no ref)
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | AA
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    [EOF]
    ");
    let pre_push_opid = work_dir.current_operation_id();
    work_dir.run_jj(["git", "push"]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @origin: qpvuntsm 75e78001 (empty) BB
    [EOF]
    ");

    // Undo the push
    work_dir.run_jj(["op", "restore", &pre_push_opid]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    [EOF]
    ");

    // PROBLEM: inserting this import changes the outcome compared to previous test
    // TODO: decide if this is the better behavior, and whether import of
    // remote-tracking bookmarks should happen on every operation.
    work_dir.run_jj(["git", "import"]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @origin: qpvuntsm 75e78001 (empty) BB
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "CC"]).success();
    work_dir.run_jj(["git", "fetch"]).success();
    // There is not a conflict. This seems like a good outcome; undoing `git push`
    // was essentially a no-op.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 20b2cc4b (empty) CC
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 75e78001 (empty) BB
    [EOF]
    ");
}

// This test is currently *identical* to `test_git_push_undo` except the repo
// it's operating it is colocated.
#[test]
fn test_git_push_undo_colocated() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path.clone());
    let work_dir = test_env.work_dir("clone");
    git::clone(work_dir.root(), git_repo_path.to_str().unwrap(), None);

    work_dir.run_jj(["git", "init", "--git-repo=."]).success();

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["describe", "-m", "AA"]).success();
    work_dir.run_jj(["git", "push", "--allow-new"]).success();
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "BB"]).success();
    //   Refs at this point look as follows (-- means no ref)
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   BB   | BB
    //    remote-tracking  | AA      |   AA   | AA
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @git: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    [EOF]
    ");
    let pre_push_opid = work_dir.current_operation_id();
    work_dir.run_jj(["git", "push"]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   BB   | BB
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @git: qpvuntsm 75e78001 (empty) BB
      @origin: qpvuntsm 75e78001 (empty) BB
    [EOF]
    ");

    // Undo the push
    work_dir.run_jj(["op", "restore", &pre_push_opid]).success();
    //       === Before auto-export ====
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   BB   | BB
    //    remote-tracking  | AA      |   BB   | BB
    //       === After automatic `jj git export` ====
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   BB   | BB
    //    remote-tracking  | AA      |   AA   | AA
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @git: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "CC"]).success();
    work_dir.run_jj(["git", "fetch"]).success();
    // We have the same conflict as `test_git_push_undo`. TODO: why did we get the
    // same result in a seemingly different way?
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main (conflicted):
      - qpvuntsm hidden 2080bdb8 (empty) AA
      + qpvuntsm?? 20b2cc4b (empty) CC
      + qpvuntsm?? 75e78001 (empty) BB
      @git (behind by 1 commits): qpvuntsm?? 20b2cc4b (empty) CC
      @origin (behind by 1 commits): qpvuntsm?? 75e78001 (empty) BB
    [EOF]
    ");
}

// This test is currently *identical* to `test_git_push_undo` except
// both the git_refs and the remote-tracking bookmarks are preserved by undo.
// TODO: Investigate the different outcome
#[test]
fn test_git_push_undo_repo_only() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(".", ["git", "clone", "git-repo", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["describe", "-m", "AA"]).success();
    work_dir.run_jj(["git", "push", "--allow-new"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 2080bdb8 (empty) AA
      @origin: qpvuntsm 2080bdb8 (empty) AA
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "BB"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    [EOF]
    ");
    let pre_push_opid = work_dir.current_operation_id();
    work_dir.run_jj(["git", "push"]).success();

    // Undo the push, but keep both the git_refs and the remote-tracking bookmarks
    work_dir
        .run_jj(["op", "restore", "--what=repo", &pre_push_opid])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 75e78001 (empty) BB
      @origin: qpvuntsm 75e78001 (empty) BB
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "CC"]).success();
    work_dir.run_jj(["git", "fetch"]).success();
    // This currently gives an identical result to `test_git_push_undo_import`.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 20b2cc4b (empty) CC
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 75e78001 (empty) BB
    [EOF]
    ");
}

#[test]
fn test_bookmark_track_untrack_undo() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(".", ["git", "clone", "git-repo", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-mcommit"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "feature1", "feature2"])
        .success();
    work_dir.run_jj(["git", "push", "--allow-new"]).success();
    work_dir
        .run_jj(["bookmark", "delete", "feature2"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
      @origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2 (deleted)
      @origin: qpvuntsm 8da1cfc8 (empty) commit
    [EOF]
    ");

    // Track/untrack can be undone so long as states can be trivially merged.
    work_dir
        .run_jj(["bookmark", "untrack", "feature1@origin", "feature2@origin"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
    feature1@origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2@origin: qpvuntsm 8da1cfc8 (empty) commit
    [EOF]
    ");

    work_dir.run_jj(["undo"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
      @origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2 (deleted)
      @origin: qpvuntsm 8da1cfc8 (empty) commit
    [EOF]
    ");

    work_dir.run_jj(["undo"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
    feature1@origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2@origin: qpvuntsm 8da1cfc8 (empty) commit
    [EOF]
    ");

    work_dir
        .run_jj(["bookmark", "track", "feature1@origin"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
      @origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2@origin: qpvuntsm 8da1cfc8 (empty) commit
    [EOF]
    ");

    work_dir.run_jj(["undo"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
    feature1@origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2@origin: qpvuntsm 8da1cfc8 (empty) commit
    [EOF]
    ");
}

#[test]
fn test_shows_a_warning_when_undoing_an_undo_operation_as_bare_jj_undo() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Double-undo creation of child
    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 2d5b73a97567 (2001-02-03 08:05:09) undo operation 289cb69a8458456474a77cc432e8009b99f039cdcaf19ba4526753e97d70fee3fd0f410ff2b7c1d10cf0c2501702e7a85d58f9d813cdca567c377431ec4d2b97
    Working copy  (@) now at: rlvkpnrz 65b6b74e (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 230dd059 (empty) (no description set)
    Hint: This action reverted an 'undo' operation. The repository is now in the same state as it was before the original 'undo'.
    Hint: If your goal is to undo multiple operations, consider using `jj op log` to see past states, and `jj op restore` to restore one of these states.
    [EOF]
    ");

    // Double-undo creation of sibling
    work_dir.run_jj(["new", "@-"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: b16799358b33 (2001-02-03 08:05:12) undo operation b14487c6d6d98f7f575ea03c48ed92d899c2a0ecbe9458221b6fc11af2bf6d918c9620cae1f8268012b0e25c7dd6f78b19ec628d0504a0830dc562d6625ba9ec
    Working copy  (@) now at: mzvwutvl 167f90e7 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 230dd059 (empty) (no description set)
    Hint: This action reverted an 'undo' operation. The repository is now in the same state as it was before the original 'undo'.
    Hint: If your goal is to undo multiple operations, consider using `jj op log` to see past states, and `jj op restore` to restore one of these states.
    [EOF]
    ");
}

#[test]
fn test_shows_no_warning_when_undoing_a_specific_undo_change() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["op", "log"]).success();
    let op_id_hex = output.stdout.raw()[3..15].to_string();
    let output = work_dir.run_jj(["undo", &op_id_hex]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 2d5b73a97567 (2001-02-03 08:05:09) undo operation 289cb69a8458456474a77cc432e8009b99f039cdcaf19ba4526753e97d70fee3fd0f410ff2b7c1d10cf0c2501702e7a85d58f9d813cdca567c377431ec4d2b97
    Working copy  (@) now at: rlvkpnrz 65b6b74e (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 230dd059 (empty) (no description set)
    [EOF]
    ");
}

#[must_use]
fn get_bookmark_output(work_dir: &TestWorkDir) -> CommandOutput {
    // --quiet to suppress deleted bookmarks hint
    work_dir.run_jj(["bookmark", "list", "--all-remotes", "--quiet"])
}
