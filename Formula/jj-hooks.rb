class JjHooks < Formula
  desc "Run pre-commit / lefthook / hk hooks against jj bookmark pushes"
  homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/jj-hooks"
  version "0.3.4"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-hooks-v#{version}-darwin-arm64.tar.gz"
      # SHA256 is bumped by .github/workflows/release.yml when a tag is pushed.
      sha256 "1d69b8fdf74bfb9633a5831a3ef953bcee15ccb73f9fada10c19549b06e2f015"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-hooks-v#{version}-linux-x64.tar.gz"
      sha256 "9afbdb18a01d9aa1b6402076a697aea4b204718a0e5b199d5e4f4f5b95a4fc2c"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-hooks-v#{version}-linux-arm64.tar.gz"
      sha256 "636338c6edbab3666c26528811ce13a5090e30d99972f84bed6a8ed5a60107ec"
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
