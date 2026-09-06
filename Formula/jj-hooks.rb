class JjHooks < Formula
  desc "Run pre-commit / lefthook / hk hooks against jj bookmark pushes"
  homepage "https://github.com/mattwilkinsonn/jj-hooks"
  version "0.3.11"
  license any_of: ["MIT", "Apache-2.0"]

  # Retired: jj-hooks now ships from the mattwilkinsonn/tap tap. The date must
  # stay strictly in the past. Homebrew hard-errors when `date <= Date.today`
  # (it is only deprecated `if disable_date > Date.today`) but renders the
  # message tense off `date < Date.today`, so ON the date it errors while
  # still saying "will be disabled". Backdating makes the two agree.
  # Needs Homebrew >= 4.4.32 for `replacement_formula:`.
  disable! date: "2026-09-04",
           because: "moved to the mattwilkinsonn/tap tap",
           replacement_formula: "mattwilkinsonn/tap/jj-hooks"

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-hooks-v#{version}-darwin-arm64.tar.gz"
      sha256 "1b5ef42e1b7418426c5f8e63fc26bc1fb976f8ef5ab9acd0a1c22bd12ef339f4"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-hooks-v#{version}-linux-x64.tar.gz"
      sha256 "29275755d1bc2c5a8e448ea3e8a69cf9c3edca1d10637a1ff0c551107823ff4d"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-hooks-v#{version}-linux-arm64.tar.gz"
      sha256 "6f14b6005ef14019c1c22ff8657e06b3afd62b18c634290de95730a4e4bc490a"
    end
  end

  def install
    bin.install "jj-hooks"
    bin.install "jj-hp"
  end

  test do
    assert_match "jj-hooks #{version}", shell_output("#{bin}/jj-hooks --version")
    assert_match "jj-hp #{version}", shell_output("#{bin}/jj-hp --version")
  end
end
