// SPDX-License-Identifier: GPL-3.0-or-later
#include "ttaplugin.h"
#include "fooyin_tta.h"

#include <QDebug>
#include <QFile>
#include <QFileInfo>
#include <QIODevice>

#include <algorithm>
#include <limits>
#include <memory>
#include <vector>

namespace
{
class TtaDecoder final : public Fooyin::AudioDecoder
{
  public:
    ~TtaDecoder() override { stop(); }

    QStringList extensions() const override { return {QStringLiteral("tta")}; }
    QStringList preferredExtensions() const override { return extensions(); }
    bool isSeekable() const override { return m_handle && !m_noSeeking; }

    const TtaStreamInfo &streamInfo() const { return m_info; }

    std::optional<Fooyin::AudioFormat> init(const Fooyin::AudioSource &source, const Fooyin::Track &track,
                                            DecoderOptions options) override
    {
        stop();
        m_noSeeking = options.testFlag(NoSeeking);
        m_device = source.device;
        if(!m_device)
        {
            m_file = std::make_unique<QFile>(source.filepath);
            if(!m_file->open(QIODevice::ReadOnly))
            {
                qWarning() << "TTA: could not open" << source.filepath << m_file->errorString();
                stop();
                return std::nullopt;
            }
            m_device = m_file.get();
        }
        if(!m_device->isReadable() || m_device->isSequential())
        {
            qWarning() << "TTA requires a readable, seekable input device";
            stop();
            return std::nullopt;
        }
        char error[512]{};
        m_handle = tta_open({this, readInput, seekInput}, &m_info, error, sizeof(error));
        if(!m_handle)
        {
            qWarning() << "TTA:" << error;
            stop();
            return std::nullopt;
        }
        m_format = Fooyin::AudioFormat{Fooyin::SampleFormat::S32, static_cast<int>(m_info.sample_rate),
                                       static_cast<int>(m_info.channels)};
        // TTA carries a channel count, but no speaker mask. Leave multichannel
        // layout unspecified rather than assigning speakers absent from the file.
        if(m_info.channels > 2)
        {
            m_format.clearChannelLayout();
        }
        // WaveBar and ReplayGain scans request NoSeeking, but still need the
        // initial CUE segment offset. Position once before disabling seeks.
        if(m_noSeeking && track.offset() > 0 && tta_seek(m_handle, sampleForTime(track.offset())) != 0)
        {
            qWarning() << "TTA:" << tta_last_error(m_handle);
            stop();
            return std::nullopt;
        }
        return m_format;
    }

    void stop() override
    {
        tta_close(m_handle);
        m_handle = nullptr;
        m_device = nullptr;
        m_file.reset();
        m_samples.clear();
        m_info = {};
        m_format = {};
        m_noSeeking = false;
    }

    void seek(uint64_t milliseconds) override
    {
        if(!isSeekable())
        {
            return;
        }
        tta_seek(m_handle, sampleForTime(milliseconds));
    }

    ReadResult readAudio(size_t bytes) override
    {
        if(!m_handle)
        {
            return ReadResult::errorResult(QStringLiteral("TTA decoder is not initialised"));
        }
        if(abortToken().stop_requested())
        {
            return ReadResult::errorResult(QStringLiteral("TTA decoding cancelled"));
        }
        const size_t stride = size_t{m_info.channels} * sizeof(int32_t);
        if(bytes < stride)
        {
            return ReadResult::errorResult(QStringLiteral("TTA output request is smaller than one sample frame"));
        }
        const size_t frames = std::min(bytes / stride, size_t{16384});
        m_samples.resize(frames * m_info.channels);
        const uint64_t timestamp = tta_position(m_handle) * 1000 / m_info.sample_rate;
        const int64_t count = tta_read(m_handle, m_samples.data(), m_samples.size());
        if(count < 0)
        {
            return ReadResult::errorResult(QString::fromUtf8(tta_last_error(m_handle)));
        }
        if(count == 0)
        {
            return ReadResult::endOfStream();
        }
        return ReadResult::data(Fooyin::AudioBuffer{reinterpret_cast<const uint8_t *>(m_samples.data()),
                                                    static_cast<size_t>(count) * sizeof(int32_t), m_format, timestamp});
    }

    Fooyin::AudioBuffer readBuffer(size_t bytes) override { return readAudio(bytes).buffer; }

  private:
    uint64_t sampleForTime(uint64_t milliseconds) const
    {
        // Clamp before multiplying, so even UINT64_MAX is a safe EOF seek.
        const uint64_t endMs = (uint64_t{m_info.total_samples} * 1000 + m_info.sample_rate - 1) / m_info.sample_rate;
        return milliseconds >= endMs ? m_info.total_samples : milliseconds * m_info.sample_rate / 1000;
    }

    static int64_t readInput(void *context, uint8_t *output, size_t size) noexcept
    {
        try
        {
            auto *self = static_cast<TtaDecoder *>(context);
            if(self->abortToken().stop_requested() || size > static_cast<size_t>(std::numeric_limits<qint64>::max()))
            {
                return -1;
            }
            return self->m_device->read(reinterpret_cast<char *>(output), static_cast<qint64>(size));
        }
        catch(...)
        {
            return -1;
        }
    }

    static int64_t seekInput(void *context, int64_t offset, int32_t origin) noexcept
    {
        try
        {
            auto *self = static_cast<TtaDecoder *>(context);
            if(self->abortToken().stop_requested())
            {
                return -1;
            }
            auto *device = self->m_device;
            qint64 base = 0;
            if(origin == 1)
            {
                base = device->pos();
            }
            else if(origin == 2)
            {
                base = device->size();
            }
            else if(origin != 0)
            {
                return -1;
            }
            if(base < 0 || offset < -base || (offset > 0 && base > std::numeric_limits<qint64>::max() - offset))
            {
                return -1;
            }
            const qint64 position = base + offset;
            return device->seek(position) ? position : -1;
        }
        catch(...)
        {
            return -1;
        }
    }

    TtaHandle *m_handle{nullptr};
    TtaStreamInfo m_info{};
    QIODevice *m_device{nullptr};
    std::unique_ptr<QFile> m_file;
    Fooyin::AudioFormat m_format;
    std::vector<int32_t> m_samples;
    bool m_noSeeking{false};
};

// Supply technical metadata when no configured tag reader handles TTA. CUE
// tracks inherit these properties; the final track needs the full file duration
// because there is no following INDEX from which to calculate its end.
class TtaReader final : public Fooyin::AudioReader
{
  public:
    QStringList extensions() const override { return {QStringLiteral("tta")}; }
    bool canReadCover() const override { return false; }
    bool canWriteMetaData() const override { return false; }

    bool init(const Fooyin::AudioSource &source) override
    {
        m_info = {};
        m_fileSize = 0;
        TtaDecoder probe;
        if(!probe.init(source, {}, Fooyin::AudioDecoder::None))
        {
            return false;
        }
        m_info = probe.streamInfo();
        const qint64 size = source.device ? source.device->size() : QFileInfo{source.filepath}.size();
        if(size > 0)
        {
            m_fileSize = static_cast<uint64_t>(size);
        }
        return true;
    }

    bool readTrack(const Fooyin::AudioSource &, Fooyin::Track &track) override
    {
        if(m_info.sample_rate == 0)
        {
            return false;
        }
        const uint64_t duration = uint64_t{m_info.total_samples} * 1000 / m_info.sample_rate;
        track.setDuration(duration);
        track.setSampleFrames(m_info.total_samples);
        track.setSampleRate(static_cast<int>(m_info.sample_rate));
        track.setChannels(static_cast<int>(m_info.channels));
        track.setBitDepth(static_cast<int>(m_info.bits_per_sample));
        track.setCodec(QStringLiteral("TTA"));
        track.setEncoding(QStringLiteral("Lossless"));
        track.setFileSize(m_fileSize);
        track.setBitrate(duration > 0 ? static_cast<int>(std::min(double(m_fileSize) * 8 / double(duration),
                                                                  double(std::numeric_limits<int>::max())))
                                      : 0);
        return true;
    }

  private:
    TtaStreamInfo m_info{};
    uint64_t m_fileSize{0};
};
} // namespace

QString TtaPlugin::inputName() const
{
    return QStringLiteral("True Audio");
}

Fooyin::InputCreator TtaPlugin::inputCreator() const
{
    Fooyin::InputCreator creator;
    creator.decoder = [] { return std::make_unique<TtaDecoder>(); };
    creator.reader = [] { return std::make_unique<TtaReader>(); };
    return creator;
}
