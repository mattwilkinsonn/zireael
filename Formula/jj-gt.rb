class JjGt < Formula
  desc "Bridge jj bookmark stacks and Graphite (gt) PR stacks"
  homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/jj-gt"
  version "0.3.2"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-darwin-arm64.tar.gz"
      # SHA256 is bumped by .github/workflows/release.yml when a tag is pushed.
      sha256 "ea3fcc1af955fe2c39f5f10ce957403479caca5c6b80d3db08523e6a1c7c7d79"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-linux-x64.tar.gz"
      sha256 "da13f311de601c3eb284c1743d0d3264f728a93e9fa9dfc0c15754c79ca5791f"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-linux-arm64.tar.gz"
      sha256 "07f6ed28a425924f6d74f83656eff088857f1074fabfbd11177c140f82137eb5"
    end
  end

  def install
    bin.install "jj-gt"
  end

  test do
    assert_match "jj-gt #{version}", shell_output("#{bin}/jj-gt --version")
  end
end
