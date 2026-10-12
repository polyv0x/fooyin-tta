// SPDX-License-Identifier: GPL-3.0-or-later
#include <core/engine/audioloader.h>
#include <core/engine/inputplugin.h>
#include <core/plugins/plugin.h>

#include <QBuffer>
#include <QCoreApplication>
#include <QFile>
#include <QPluginLoader>
#include <QtEndian>

#include <cstring>
#include <iostream>
#include <limits>
#include <stdexcept>

namespace
{
void check(bool condition, const char *message)
{
    if(!condition)
    {
        throw std::runtime_error(message);
    }
}

QByteArray load(const QString &path)
{
    QFile file{path};
    check(file.open(QIODevice::ReadOnly), "could not open test fixture");
    return file.readAll();
}

void compare(const Fooyin::AudioBuffer &buffer, const QByteArray &expected, size_t offset)
{
    check(buffer.isValid(), "invalid PCM buffer");
    const auto bytes = buffer.constData();
    check(offset + bytes.size() <= static_cast<size_t>(expected.size()), "too many output samples");
    check(bytes.size() % 4 == 0, "unaligned S32 buffer");
    for(size_t index = 0; index < bytes.size(); index += 4)
    {
        int32_t actual{};
        std::memcpy(&actual, bytes.data() + index, 4);
        const auto reference = qFromLittleEndian<int32_t>(expected.constData() + offset + index);
        check(actual == reference, "PCM mismatch");
    }
}

// Exercise short reads as well as a normal in-memory archive-like source.
class ShortReadBuffer final : public QBuffer
{
  protected:
    qint64 readData(char *data, qint64 size) override { return QBuffer::readData(data, std::min(size, qint64{7})); }
};

void verifyDecoder(const Fooyin::InputCreator &creator, const QString &directory, const QString &name,
                   bool deviceSource)
{
    const QString path = directory + '/' + name + QStringLiteral(".tta");
    const QByteArray expected = load(directory + '/' + name + QStringLiteral(".s32le"));
    ShortReadBuffer input;
    input.setData(load(path));
    check(input.open(QIODevice::ReadOnly | QIODevice::Unbuffered), "could not open input buffer");
    Fooyin::AudioSource source;
    // A nonexistent filename proves supplied QIODevices are used, not reopened.
    source.filepath = deviceSource ? QStringLiteral("archive://does-not-exist/track.tta") : path;
    source.device = deviceSource ? &input : nullptr;
    auto decoder = creator.decoder();
    const Fooyin::Track track;
    check(decoder->extensions().contains(QStringLiteral("tta")), "TTA extension missing");
    check(decoder->preferredExtensions().contains(QStringLiteral("tta")), "decoder is not preferred for TTA");
    const auto format = decoder->init(source, track, Fooyin::AudioDecoder::None);
    check(format && format->isValid(), "plugin init failed");
    check(format->sampleFormat() == Fooyin::SampleFormat::S32, "wrong output format");
    check(decoder->isSeekable(), "seeking disabled unexpectedly");
    const auto stride = static_cast<size_t>(format->bytesPerFrame());
    size_t offset = 0;
    while(true)
    {
        auto result = decoder->readAudio(413 * stride + 1);
        if(result.status == Fooyin::AudioDecoder::ReadStatus::EndOfStream)
        {
            break;
        }
        if(result.status == Fooyin::AudioDecoder::ReadStatus::Error)
        {
            throw std::runtime_error(result.error.toStdString());
        }
        check(result.status == Fooyin::AudioDecoder::ReadStatus::DecodedAudio, "unexpected read status");
        check(result.buffer.startTime() == (offset / stride) * 1000 / format->sampleRate(), "incorrect timestamp");
        check(static_cast<size_t>(result.buffer.byteCount()) <= 413 * stride + 1, "exceeded read size");
        compare(result.buffer, expected, offset);
        offset += result.buffer.byteCount();
    }
    check(offset == static_cast<size_t>(expected.size()), "lost final samples");
    for(uint64_t milliseconds : {0ULL, 13ULL, 1044ULL, 1045ULL, 1046ULL, 1ULL})
    {
        decoder->seek(milliseconds);
        const auto sample = std::min<uint64_t>(milliseconds * format->sampleRate() / 1000, expected.size() / stride);
        const auto result = decoder->readAudio(37 * stride);
        if(sample == static_cast<uint64_t>(expected.size()) / stride)
        {
            check(result.status == Fooyin::AudioDecoder::ReadStatus::EndOfStream, "EOF seek returned audio");
        }
        else
        {
            check(result.status == Fooyin::AudioDecoder::ReadStatus::DecodedAudio, "seek did not return audio");
            check(result.buffer.startTime() == sample * 1000 / format->sampleRate(), "wrong timestamp after seek");
            compare(result.buffer, expected, sample * stride);
        }
    }
    decoder->seek(std::numeric_limits<uint64_t>::max());
    check(decoder->readAudio(4096).status == Fooyin::AudioDecoder::ReadStatus::EndOfStream, "large seek overflowed");
    decoder->seek(0);
    compare(decoder->readBuffer(37 * stride), expected, 0);
    decoder->stop();
    decoder->stop();
    check(!decoder->isSeekable(), "stopped decoder is seekable");
    check(input.isOpen(), "plugin closed borrowed input");
    check(decoder->init(source, track, Fooyin::AudioDecoder::NoSeeking).has_value(), "reinitialisation failed");
    check(!decoder->isSeekable(), "NoSeeking ignored");
    decoder->seek(13);
    compare(decoder->readBuffer(37 * stride), expected, 0);
    decoder->requestAbort();
    check(decoder->readAudio(4096).status == Fooyin::AudioDecoder::ReadStatus::Error, "abort ignored");
}

void verifyErrors(const Fooyin::InputCreator &creator, const QString &directory)
{
    QByteArray data = load(directory + QStringLiteral("/mono8.tta"));
    data[32] = static_cast<char>(data.at(32) ^ 1);
    QBuffer input{&data};
    check(input.open(QIODevice::ReadOnly), "could not open corrupt fixture");
    Fooyin::AudioSource source;
    source.device = &input;
    auto decoder = creator.decoder();
    const Fooyin::Track track;
    check(decoder->init(source, track, Fooyin::AudioDecoder::None).has_value(), "valid header rejected");
    const auto result = decoder->readAudio(4096);
    check(result.status == Fooyin::AudioDecoder::ReadStatus::Error, "corrupt data treated as EOF/audio");
    check(result.error.contains(QStringLiteral("checksum")), "missing checksum diagnostic");
    check(decoder->readAudio(4096).status == Fooyin::AudioDecoder::ReadStatus::Error, "error was not latched");
    decoder->stop();
    input.close();
    data[0] = 'X';
    input.open(QIODevice::ReadOnly);
    check(!decoder->init(source, track, Fooyin::AudioDecoder::None).has_value(), "invalid header accepted");
    check(!decoder->isSeekable(), "failed init retained old decoder");
}

void verifyReader(const Fooyin::InputCreator &creator, const QString &directory, bool deviceSource)
{
    const QString path = directory + QStringLiteral("/stereo16.tta");
    const auto pcm = load(directory + QStringLiteral("/stereo16.s32le"));
    ShortReadBuffer input;
    input.setData(load(path));
    check(input.open(QIODevice::ReadOnly | QIODevice::Unbuffered), "could not open metadata source");
    Fooyin::AudioSource source;
    source.filepath = deviceSource ? QStringLiteral("archive://missing/album.tta") : path;
    source.device = deviceSource ? &input : nullptr;
    auto reader = creator.reader();
    Fooyin::Track track{path};
    track.setTitle(QStringLiteral("CUE title"));
    track.setArtists({QStringLiteral("CUE artist")});
    check(!reader->readTrack(source, track), "uninitialised reader succeeded");
    check(reader->preferredExtensions().isEmpty(), "technical reader overrides configured tag readers");
    check(reader->init(source) && reader->readTrack(source, track), "TTA metadata read failed");
    check(track.sampleFrames() == static_cast<uint64_t>(pcm.size()) / 8, "incorrect total sample count");
    check(track.duration() == 2094, "missing or incorrect TTA duration");
    check(track.sampleRate() == 8000 && track.channels() == 2 && track.bitDepth() == 16,
          "incorrect TTA audio properties");
    check(track.fileSize() == static_cast<uint64_t>(input.size()) && track.bitrate() > 0,
          "missing TTA file properties");
    check(track.codec() == QStringLiteral("TTA") && track.encoding() == QStringLiteral("Lossless"),
          "incorrect codec metadata");
    check(track.title() == QStringLiteral("CUE title") && track.artists() == QStringList{QStringLiteral("CUE artist")},
          "technical reader overwrote tags");
    check(input.isOpen(), "metadata reader closed borrowed input");

    QByteArray invalid{"not a TTA file"};
    QBuffer bad{&invalid};
    check(bad.open(QIODevice::ReadOnly), "could not open invalid metadata input");
    source.device = &bad;
    check(!reader->init(source), "metadata reader accepted invalid TTA");
    check(!reader->readTrack(source, track), "failed metadata init retained previous properties");
}

void verifyCueScan(const Fooyin::InputCreator &creator, const QString &directory)
{
    Fooyin::AudioLoader loader;
    loader.addReader(QStringLiteral("True Audio"), creator.reader);
    loader.addDecoder(QStringLiteral("True Audio"), creator.decoder);
    Fooyin::Track image{directory + QStringLiteral("/stereo16.tta")};
    check(loader.readTrackMetadata(image), "registry could not obtain TTA image duration");

    // A final CUE track has no following INDEX: fooyin derives its duration
    // from the image metadata, then scans that segment with NoSeeking set.
    Fooyin::Track last{image};
    last.setCuePath(directory + QStringLiteral("/album.cue"));
    last.setOffset(1000);
    check(image.duration() > last.offset(), "final CUE track has unknown duration");
    last.setDuration(image.duration() - last.offset());
    check(last.duration() == 1094, "incorrect final CUE track duration");
    auto loaded =
        loader.loadDecoderForTrack(last, Fooyin::AudioDecoder::NoSeeking | Fooyin::AudioDecoder::NoInfiniteLooping);
    check(loaded.decoder != nullptr && loaded.format.has_value(), "could not load CUE waveform decoder");
    loaded.decoder->start();
    loaded.decoder->seek(last.offset());
    const auto expected = load(directory + QStringLiteral("/stereo16.s32le"));
    const auto stride = static_cast<size_t>(loaded.format->bytesPerFrame());
    const auto startByte = last.offset() * loaded.format->sampleRate() / 1000 * stride;
    size_t decoded = 0;
    while(true)
    {
        const auto result = loaded.decoder->readAudio(4096);
        if(result.status == Fooyin::AudioDecoder::ReadStatus::EndOfStream)
        {
            break;
        }
        check(result.status == Fooyin::AudioDecoder::ReadStatus::DecodedAudio, "CUE waveform decoding failed");
        check(result.buffer.startTime() == (startByte + decoded) / stride * 1000 / loaded.format->sampleRate(),
              "CUE scan started at the wrong timestamp");
        compare(result.buffer, expected, startByte + decoded);
        decoded += result.buffer.byteCount();
    }
    check(decoded == static_cast<size_t>(expected.size()) - startByte, "CUE scan did not cover the final track");
    check(decoded >= loaded.format->bytesForDuration(last.duration()), "not enough audio for final waveform");
}

void verifyRegistry(Fooyin::InputPlugin *plugin, const QString &directory)
{
    Fooyin::AudioLoader loader;
    loader.addDecoder(plugin->inputName(), plugin->inputCreator().decoder);
    loader.addReader(plugin->inputName(), plugin->inputCreator().reader);
    const QString path = directory + QStringLiteral("/stereo16.tta");
    Fooyin::Track track{path};
    check(loader.supportedTrackExtensions().contains(QStringLiteral("tta")), "TTA not advertised by registry");
    check(loader.readTrackMetadata(track) && track.duration() > 0, "registry failed to read TTA metadata");
    auto loaded = loader.loadDecoderForTrack(track);
    check(loaded.decoder != nullptr, "fooyin registry failed to open TTA");
    check(loaded.name == plugin->inputName(), "fooyin selected a different TTA decoder");
    compare(loaded.decoder->readBuffer(4096), load(directory + QStringLiteral("/stereo16.s32le")), 0);
}
} // namespace

int main(int argc, char **argv)
{
    QCoreApplication app{argc, argv};
    if(argc != 3)
    {
        std::cerr << "usage: tta_plugin_test PLUGIN FIXTURE_DIRECTORY\n";
        return 2;
    }
    try
    {
        QPluginLoader loader{QString::fromLocal8Bit(argv[1])};
        auto *object = loader.instance();
        if(!object)
        {
            throw std::runtime_error(loader.errorString().toStdString());
        }
        check(qobject_cast<Fooyin::Plugin *>(object) != nullptr, "not a fooyin plugin");
        auto *plugin = qobject_cast<Fooyin::InputPlugin *>(object);
        check(plugin != nullptr, "missing fooyin input interface");
        check(plugin->inputName() == QStringLiteral("True Audio"), "wrong plugin identity");
        const auto creator = plugin->inputCreator();
        check(static_cast<bool>(creator.decoder), "missing decoder factory");
        check(static_cast<bool>(creator.reader), "missing metadata reader factory");
        const QString directory = QString::fromLocal8Bit(argv[2]);
        for(const auto *name : {"mono8", "stereo8", "stereo16", "surround24", "silence16"})
        {
            for(bool deviceSource : {false, true})
            {
                verifyDecoder(creator, directory, QString::fromLatin1(name), deviceSource);
            }
        }
        verifyErrors(creator, directory);
        verifyReader(creator, directory, false);
        verifyReader(creator, directory, true);
        verifyCueScan(creator, directory);
        verifyRegistry(plugin, directory);
        std::cout << "Plugin loaded; metadata, CUE scans, PCM, timestamps, seeking, EOF, cancellation, reinit, device "
                     "ownership, and error "
                     "handling passed\n";
        return 0;
    }
    catch(const std::exception &error)
    {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
