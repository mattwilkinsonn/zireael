class JjGt < Formula
  desc "Bridge jj bookmark stacks and Graphite (gt) PR stacks"
  homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/jj-gt"
  version "0.3.9"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-darwin-arm64.tar.gz"
      # SHA256 is bumped by .github/workflows/release.yml when a tag is pushed.
      sha256 "75ae7a3c5e8c11280a79764ed25801fefeb08c7a27d65ef810c6dda10de0e137"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-linux-x64.tar.gz"
      sha256 "7640845ed140443158e7dfd49d9f02d5355a83bfc2e619f4e3837407958052f3"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-linux-arm64.tar.gz"
      sha256 "5c828bbcfc05a384cd2775583f3c6692c1a57b473d8c10bbe1d3f5084eb8a58a"
    end
  end

  def install
    bin.install "jj-gt"
  end

  test do
    assert_match "jj-gt #{version}", shell_output("#{bin}/jj-gt --version")
  end
end
