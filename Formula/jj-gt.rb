class JjGt < Formula
  desc "Bridge jj bookmark stacks and Graphite (gt) PR stacks"
  homepage "https://github.com/mattwilkinsonn/jj-gt"
  version "0.3.11"
  license any_of: ["MIT", "Apache-2.0"]

  # Retired: jj-gt now ships from the mattwilkinsonn/tap tap. The date must
  # stay in the past: Homebrew only hard-errors once `date < Date.today`, and
  # on the date itself it still says "will be disabled".
  disable! date: "2026-09-04",
           because: "moved to the mattwilkinsonn/tap tap",
           replacement_formula: "mattwilkinsonn/tap/jj-gt"

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-darwin-arm64.tar.gz"
      sha256 "774fc0ef45d65e2c6238838c0f9f6b98cd43f619e37010f91046ed76852941c3"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-linux-x64.tar.gz"
      sha256 "8287b00cb3476ac4c8ce6a729abffd6502885f932423e939719c7851f62a7f7c"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-linux-arm64.tar.gz"
      sha256 "3ff3995c0ec2f907f55336112ba4e61f982c903ff58d09ec623e18192351690e"
    end
  end

  def install
    bin.install "jj-gt"
  end

  test do
    assert_match "jj-gt #{version}", shell_output("#{bin}/jj-gt --version")
  end
end
