# GitHub Actions workflows

These are the project's CI and release workflows. They live here rather than
in `.github/workflows/` because GitHub refuses a push that writes a workflow
file unless the pushing token carries the `workflow` scope, and the initial
import was made with a token that did not.

To switch CI on, move them into place:

    mkdir -p .github/workflows
    git mv ci/github-workflows/ci.yml      .github/workflows/ci.yml
    git mv ci/github-workflows/release.yml .github/workflows/release.yml
    git rm ci/github-workflows/README.md
    git commit -m "Enable CI"
    git push

That push needs the scope, which `gh auth refresh -h github.com -s workflow`
adds. Moving the files through GitHub's own web editor works without it.
