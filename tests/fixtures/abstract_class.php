<?php
namespace Magento\Framework\App\Action;

abstract class AbstractAction implements \Magento\Framework\App\ActionInterface
{
    public function __construct(
        protected readonly \Magento\Framework\App\RequestInterface $request,
        protected readonly \Magento\Framework\App\ResponseInterface $response
    ) {}

    abstract public function execute(): \Magento\Framework\Controller\ResultInterface;

    public function getRequest(): \Magento\Framework\App\RequestInterface
    {
        return $this->request;
    }
}
